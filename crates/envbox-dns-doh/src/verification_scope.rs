//! Caller-thread cancellation for a Send + Sync TLS verifier.
//!
//! The query owns a non-Send RAII scope until its runtime and tasks are gone.
//! Verifiers retain only an opaque identity; callback/context stay in that
//! caller thread's registry. No callback can be invoked through a retired or
//! foreign-thread identity, and no registry borrow spans a caller callback.
use crate::{Budget, Error};
use std::{cell::RefCell, marker::PhantomData, rc::Rc, thread::ThreadId};

#[derive(Default)]
struct Registry {
    next_id: u64,
    active: Vec<Active>,
}
struct Active {
    id: u64,
    budget: Budget,
    stop: Option<Error>,
    #[cfg(any(test, feature = "fixture-trust"))]
    cache_depth: usize,
}
thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}
const MAX_SCOPES: usize = 16;

pub(crate) struct VerificationScope {
    id: Option<u64>,
    _caller_thread: PhantomData<Rc<()>>,
}
impl VerificationScope {
    pub(crate) fn enter(budget: Budget) -> Result<Self, Error> {
        budget.check()?;
        let id = if budget.cancelled.is_some() {
            Some(
                REGISTRY
                    .try_with(|registry| {
                        let mut registry = registry.borrow_mut();
                        if registry.active.len() >= MAX_SCOPES {
                            return Err(Error::TrustSnapshot);
                        }
                        let id = registry
                            .next_id
                            .checked_add(1)
                            .ok_or(Error::TrustSnapshot)?;
                        registry.next_id = id;
                        registry.active.push(Active {
                            id,
                            budget,
                            stop: None,
                            #[cfg(any(test, feature = "fixture-trust"))]
                            cache_depth: 0,
                        });
                        Ok(id)
                    })
                    .map_err(|_| Error::TrustSnapshot)??,
            )
        } else {
            None
        };
        Ok(Self {
            id,
            _caller_thread: PhantomData,
        })
    }

    pub(crate) fn stop_reason(&self) -> Option<Error> {
        let id = self.id?;
        REGISTRY
            .try_with(|registry| {
                registry
                    .borrow()
                    .active
                    .iter()
                    .find(|entry| entry.id == id)
                    .and_then(|entry| entry.stop)
            })
            .ok()
            .flatten()
    }
}
impl Drop for VerificationScope {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            // All borrows end before externally supplied code runs. A borrow
            // violation is an internal invariant failure, never silently keep
            // a callback whose query has returned.
            let _ = REGISTRY.try_with(|registry| {
                registry.borrow_mut().active.retain(|entry| entry.id != id);
            });
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum VerificationBudget {
    Deadline(u64),
    Scoped { thread: ThreadId, id: u64 },
}
impl VerificationBudget {
    pub(crate) fn capture(budget: Budget) -> Result<Self, Error> {
        let Some(callback) = budget.cancelled else {
            return Ok(Self::Deadline(budget.deadline));
        };
        let id = REGISTRY
            .try_with(|registry| {
                registry
                    .borrow()
                    .active
                    .iter()
                    .rev()
                    .find_map(|active| {
                        (active.budget.deadline == budget.deadline
                            && active.budget.context == budget.context
                            && active
                                .budget
                                .cancelled
                                .is_some_and(|other| std::ptr::fn_addr_eq(other, callback)))
                        .then_some(active.id)
                    })
                    .ok_or(Error::TrustSnapshot)
            })
            .map_err(|_| Error::TrustSnapshot)??;
        Ok(Self::Scoped {
            thread: std::thread::current().id(),
            id,
        })
    }

    fn budget(self) -> Result<Budget, Error> {
        match self {
            Self::Deadline(deadline) => Ok(Budget::until(deadline)),
            Self::Scoped { thread, id } => {
                if std::thread::current().id() != thread {
                    return Err(Error::TrustSnapshot);
                }
                REGISTRY
                    .try_with(|registry| {
                        registry
                            .borrow()
                            .active
                            .iter()
                            .find_map(|entry| {
                                (entry.id == id).then_some(match entry.stop {
                                    Some(error) => Err(error),
                                    None => Ok(entry.budget),
                                })
                            })
                            .ok_or(Error::TrustSnapshot)
                    })
                    .map_err(|_| Error::TrustSnapshot)??
            }
        }
    }

    fn record_stop(self, error: Error) {
        if !matches!(error, Error::Cancelled | Error::Deadline) {
            return;
        }
        if let Self::Scoped { thread, id } = self {
            if std::thread::current().id() == thread {
                let _ = REGISTRY.try_with(|registry| {
                    if let Some(entry) = registry
                        .borrow_mut()
                        .active
                        .iter_mut()
                        .find(|entry| entry.id == id)
                    {
                        entry.stop.get_or_insert(error);
                    }
                });
            }
        }
    }

    // This synchronous closure completes while the query scope is active. Its
    // only returned data is owned DER; callback-bearing budgets cannot escape.
    pub(crate) fn collect(
        self,
        collect: impl FnOnce(Budget) -> Result<Vec<Vec<u8>>, Error>,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let budget = self.budget()?;
        #[cfg(any(test, feature = "fixture-trust"))]
        let _phase = CachePhase::enter(self)?;
        let result = (|| {
            budget.check()?;
            let result = collect(budget);
            // Preserve a stop already reported by the collector, even if a
            // caller supplies a one-shot cancellation notification.
            if matches!(result, Err(Error::Cancelled | Error::Deadline)) {
                return result;
            }
            budget.check()?;
            result
        })();
        if let Err(error) = result {
            self.record_stop(error);
        }
        result
    }
}

// Standalone fixture instrumentation identifies the callback phase without
// timing guesses. No phase input/export exists in product builds or the C ABI.
#[cfg(any(test, feature = "fixture-trust"))]
pub(crate) fn cache_collection_active() -> bool {
    REGISTRY
        .try_with(|registry| {
            registry
                .borrow()
                .active
                .last()
                .is_some_and(|entry| entry.cache_depth != 0)
        })
        .unwrap_or(false)
}
#[cfg(any(test, feature = "fixture-trust"))]
struct CachePhase {
    id: Option<u64>,
    previous: usize,
    _caller_thread: PhantomData<Rc<()>>,
}
#[cfg(any(test, feature = "fixture-trust"))]
impl CachePhase {
    fn enter(token: VerificationBudget) -> Result<Self, Error> {
        let mut previous = 0;
        let id = if let VerificationBudget::Scoped { id, .. } = token {
            REGISTRY
                .try_with(|registry| {
                    let mut registry = registry.borrow_mut();
                    let entry = registry
                        .active
                        .iter_mut()
                        .find(|entry| entry.id == id)
                        .ok_or(Error::TrustSnapshot)?;
                    previous = entry.cache_depth;
                    entry.cache_depth = previous
                        .checked_add(1)
                        .filter(|value| *value <= MAX_SCOPES)
                        .ok_or(Error::TrustSnapshot)?;
                    Ok::<(), Error>(())
                })
                .map_err(|_| Error::TrustSnapshot)??;
            Some(id)
        } else {
            None
        };
        Ok(Self {
            id,
            previous,
            _caller_thread: PhantomData,
        })
    }
}
#[cfg(any(test, feature = "fixture-trust"))]
impl Drop for CachePhase {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            let _ = REGISTRY.try_with(|registry| {
                if let Some(entry) = registry
                    .borrow_mut()
                    .active
                    .iter_mut()
                    .find(|entry| entry.id == id)
                {
                    entry.cache_depth = self.previous;
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, ffi::c_void};

    struct Cancellation {
        cancelled: Cell<bool>,
        calls: Cell<usize>,
        thread: ThreadId,
    }
    unsafe extern "C" fn cancelled(context: *mut c_void) -> i32 {
        let state = &*context.cast::<Cancellation>();
        assert_eq!(std::thread::current().id(), state.thread);
        state.calls.set(state.calls.get() + 1);
        i32::from(state.cancelled.get())
    }
    fn state() -> Cancellation {
        Cancellation {
            cancelled: Cell::new(false),
            calls: Cell::new(0),
            thread: std::thread::current().id(),
        }
    }
    fn budget(state: &Cancellation) -> Budget {
        unsafe {
            Budget::from_callback(
                u64::MAX,
                Some(cancelled),
                (state as *const Cancellation).cast_mut().cast(),
            )
        }
    }

    #[test]
    fn callback_stays_on_caller_thread_and_observes_cancellation() {
        let state = state();
        let scope = VerificationScope::enter(budget(&state)).unwrap();
        let token = VerificationBudget::capture(budget(&state)).unwrap();
        assert_eq!(token.collect(|_| Ok(vec![vec![1]])), Ok(vec![vec![1]]));
        state.cancelled.set(true);
        assert_eq!(
            token.collect(|_| panic!("must not collect")),
            Err(Error::Cancelled)
        );
        drop(scope);
    }

    #[test]
    fn retired_or_foreign_tokens_never_invoke_callback() {
        let state = state();
        let scope = VerificationScope::enter(budget(&state)).unwrap();
        let token = VerificationBudget::capture(budget(&state)).unwrap();
        let before = state.calls.get();
        assert_eq!(
            std::thread::spawn(move || token.collect(|_| panic!("foreign thread")))
                .join()
                .unwrap(),
            Err(Error::TrustSnapshot)
        );
        assert_eq!(state.calls.get(), before);
        drop(scope);
        assert_eq!(
            token.collect(|_| panic!("retired scope")),
            Err(Error::TrustSnapshot)
        );
        assert_eq!(state.calls.get(), before);
    }

    #[test]
    fn nested_scopes_restore_identity_and_do_not_resurrect_retired_tokens() {
        let state = state();
        let outer = VerificationScope::enter(budget(&state)).unwrap();
        let outer_token = VerificationBudget::capture(budget(&state)).unwrap();
        let inner = VerificationScope::enter(budget(&state)).unwrap();
        let inner_token = VerificationBudget::capture(budget(&state)).unwrap();
        drop(inner);
        let replacement = VerificationScope::enter(budget(&state)).unwrap();
        assert_eq!(
            inner_token.collect(|_| Ok(vec![])),
            Err(Error::TrustSnapshot)
        );
        assert_eq!(outer_token.collect(|_| Ok(vec![])), Ok(vec![]));
        drop(replacement);
        drop(outer);
    }

    #[test]
    fn unwind_retires_scope_and_callback_requires_matching_registration() {
        let state = state();
        assert!(matches!(
            VerificationBudget::capture(budget(&state)),
            Err(Error::TrustSnapshot)
        ));
        let saved = Cell::new(None);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _scope = VerificationScope::enter(budget(&state)).unwrap();
            saved.set(Some(VerificationBudget::capture(budget(&state)).unwrap()));
            panic!("controlled unwind");
        }));
        assert!(result.is_err());
        let before = state.calls.get();
        assert_eq!(
            saved.get().unwrap().collect(|_| Ok(vec![])),
            Err(Error::TrustSnapshot)
        );
        assert_eq!(state.calls.get(), before);
    }

    #[test]
    fn one_shot_cancellation_is_latched_until_query_cleanup() {
        unsafe extern "C" fn pulse(context: *mut c_void) -> i32 {
            i32::from((&*context.cast::<Cell<bool>>()).replace(false))
        }
        let pulse_flag = Cell::new(false);
        let budget = unsafe {
            Budget::from_callback(
                u64::MAX,
                Some(pulse),
                (&pulse_flag as *const Cell<bool>).cast_mut().cast(),
            )
        };
        let scope = VerificationScope::enter(budget).unwrap();
        let token = VerificationBudget::capture(budget).unwrap();
        pulse_flag.set(true);
        assert_eq!(
            token.collect(|_| panic!("cancelled collection")),
            Err(Error::Cancelled)
        );
        assert_eq!(budget.check(), Ok(())); // Caller notification is now cleared.
        assert_eq!(scope.stop_reason(), Some(Error::Cancelled));
        assert_eq!(
            token.collect(|_| panic!("stop must remain latched")),
            Err(Error::Cancelled)
        );
    }

    #[test]
    fn callback_can_register_nested_scope_without_holding_a_registry_borrow() {
        struct Reentry {
            entered: Cell<bool>,
            succeeded: Cell<bool>,
        }
        unsafe extern "C" fn never_cancel(_: *mut c_void) -> i32 {
            0
        }
        unsafe extern "C" fn reenter(context: *mut c_void) -> i32 {
            let state = &*context.cast::<Reentry>();
            if !state.entered.replace(true) {
                let budget =
                    Budget::from_callback(u64::MAX, Some(never_cancel), std::ptr::null_mut());
                let result = (|| {
                    let _scope = VerificationScope::enter(budget)?;
                    VerificationBudget::capture(budget)?.collect(|_| Ok(vec![]))
                })();
                state.succeeded.set(result.is_ok());
            }
            0
        }
        let state = Reentry {
            entered: Cell::new(true),
            succeeded: Cell::new(false),
        };
        let budget = unsafe {
            Budget::from_callback(
                u64::MAX,
                Some(reenter),
                (&state as *const Reentry).cast_mut().cast(),
            )
        };
        let _scope = VerificationScope::enter(budget).unwrap();
        let token = VerificationBudget::capture(budget).unwrap();
        state.entered.set(false);
        assert_eq!(token.collect(|_| Ok(vec![vec![1]])), Ok(vec![vec![1]]));
        assert!(state.succeeded.get());
        assert!(!cache_collection_active());
    }
}
