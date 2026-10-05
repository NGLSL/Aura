use crate::Error;
use std::ffi::c_void;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;

pub type CancelCallback = Option<unsafe extern "C" fn(*mut c_void) -> i32>;

#[derive(Clone, Copy)]
pub struct Budget {
    pub(crate) deadline: u64,
    pub(crate) cancelled: CancelCallback,
    pub(crate) context: *mut c_void,
}

impl Budget {
    /// An absolute Windows GetTickCount64 deadline without cancellation.
    pub fn until(deadline: u64) -> Self {
        Self {
            deadline,
            cancelled: None,
            context: std::ptr::null_mut(),
        }
    }
    /// Construct a budget with a caller-owned cancellation callback.
    ///
    /// # Safety
    /// Callback and context must remain valid for every use of this budget and
    /// its copies, including all queries and trust snapshot construction. The
    /// callback must accept this context and must never unwind.
    pub unsafe fn from_callback(
        deadline: u64,
        cancelled: CancelCallback,
        context: *mut c_void,
    ) -> Self {
        Self {
            deadline,
            cancelled,
            context,
        }
    }
    pub(crate) fn check(&self) -> Result<(), Error> {
        if self
            .cancelled
            .is_some_and(|callback| unsafe { callback(self.context) != 0 })
        {
            return Err(Error::Cancelled);
        }
        if unsafe { GetTickCount64() } >= self.deadline {
            return Err(Error::Deadline);
        }
        Ok(())
    }
    pub(crate) async fn stopped(self) -> Error {
        loop {
            if let Err(error) = self.check() {
                return error;
            }
            let remaining = self.deadline.saturating_sub(unsafe { GetTickCount64() });
            tokio::time::sleep(std::time::Duration::from_millis(remaining.min(10))).await;
        }
    }
}
