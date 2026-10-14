// CIM cmdlets use Microsoft.Management.Infrastructure, whose public MI instance
// function table is also used after asynchronous results have been cloned.
// Install those seams only after MI initialization: MI must not be initialized
// while the Runtime is attaching under the Windows loader lock.
#include "hooks.h"
#include "audit.h"
#include <mi.h>
#include <tlhelp32.h>
#include <wchar.h>
#include <vector>
#include <memory>

static auto TrueInitialize = &MI_Application_InitializeV1;
static decltype(MI_InstanceFT::GetElement) TrueGetElement = nullptr;
static decltype(MI_InstanceFT::GetElementAt) TrueGetElementAt = nullptr;
static decltype(MI_InstanceFT::Clone) TrueClone = nullptr;
static decltype(MI_ApplicationFT::NewSession) TrueNewSession = nullptr;
static decltype(MI_SessionFT::Close) TrueSessionClose = nullptr;
static decltype(MI_SessionFT::GetInstance) TrueSessionGet = nullptr;
static decltype(MI_SessionFT::EnumerateInstances) TrueEnumerate = nullptr;
static decltype(MI_SessionFT::QueryInstances) TrueQuery = nullptr;
static decltype(MI_SessionFT::AssociatorInstances) TrueAssociator = nullptr;
static decltype(MI_SessionFT::ReferenceInstances) TrueReference = nullptr;
static decltype(MI_OperationFT::GetInstance) TrueOperationGet = nullptr;
static decltype(MI_OperationFT::Close) TrueOperationClose = nullptr;
static INIT_ONCE g_once = INIT_ONCE_STATIC_INIT;
static bool g_ready = false;
static wchar_t g_host[3][256] = {};
static wchar_t g_colon[18] = {}, g_dash[18] = {}, g_plain[13] = {};
static MI_Char* g_single_address[] = {g_plain};
struct CloseHandleDeleter {
  void operator()(void* handle) const {
    if (handle && handle != INVALID_HANDLE_VALUE) CloseHandle(handle);
  }
};
using OwnedHandle = std::unique_ptr<void, CloseHandleDeleter>;
struct DetourTransactionGuard {
  bool active = true;
  ~DetourTransactionGuard() { if (active) DetourTransactionAbort(); }
};

// MI copies its public handle structs; the opaque identity values remain stable.
// Fixed registries bound memory and retain only live sessions/operations. No
// native call or user callback executes while their lock is held.
struct Key {
  MI_Uint64 first = 0; ptrdiff_t second = 0;
  template<class T> explicit Key(const T* p) : first(p ? p->reserved1 : 0),
      second(p ? p->reserved2 : 0) {}
  Key() = default;
  bool operator==(const Key& other) const {
    return first == other.first && second == other.second;
  }
};
struct SessionSource { bool active = false; Key key; };
struct OperationSource {
  bool active = false;
  unsigned long long generation = 0;
  Key key;
  const MI_Instance* borrowed = nullptr;
  bool asynchronous = false, completed = false, close_requested = false;
  bool acknowledgement_pending = false;
  unsigned long long acknowledgement_epoch = 0;
  MI_Result (MI_CALL* native_acknowledgement)(MI_Operation*) = nullptr;
  unsigned callbacks_running = 0, calls_running = 0;
  MI_OperationCallbacks original = {}, wrapped = {};
};
static SRWLOCK g_sources_lock = SRWLOCK_INIT;
static SessionSource g_sessions[256];
static OperationSource g_operations[1024];
struct SourceLock {
  SourceLock() { AcquireSRWLockExclusive(&g_sources_lock); }
  ~SourceLock() { ReleaseSRWLockExclusive(&g_sources_lock); }
};
static void RecycleOperation(OperationSource& source) {
  if (source.close_requested && (!source.asynchronous || source.completed) &&
      !source.callbacks_running && !source.calls_running &&
      !source.acknowledgement_pending) {
    source.active = false;
    source.borrowed = nullptr;
  }
}
struct CallbackLease {
  OperationSource* source;
  MI_OperationCallbacks original;
  unsigned long long generation;
  explicit CallbackLease(OperationSource* value) : source(value) {
    SourceLock lock;
    ++source->callbacks_running;
    original = source->original;
    generation = source->generation;
  }
  ~CallbackLease() {
    SourceLock lock;
    --source->callbacks_running;
    RecycleOperation(*source);
  }
};
static thread_local const MI_Instance* g_callback_instance = nullptr;
struct CallbackSource {
  const MI_Instance* previous;
  explicit CallbackSource(const MI_Instance* instance)
      : previous(g_callback_instance) { g_callback_instance = instance; }
  ~CallbackSource() { g_callback_instance = previous; }
};
static bool LocalSource(const MI_Instance* instance) {
  if (!instance) return false;
  if (g_callback_instance == instance) return true;
  SourceLock lock;
  for (const auto& source : g_operations)
    if (source.active && source.borrowed == instance) return true;
  return false;
}
static bool LocalSession(const MI_Session* session) {
  if (!session) return false;
  SourceLock lock;
  for (const auto& source : g_sessions)
    if (source.active && source.key == Key(session)) return true;
  return false;
}

static bool Same(const wchar_t* a, const wchar_t* b) {
  return a && b && _wcsicmp(a, b) == 0;
}

static bool NetworkInstance(const MI_Instance* instance, bool* net_adapter) {
  if (!instance || !instance->ft) return false;
  const MI_Char *name = nullptr, *space = nullptr;
  if (MI_Instance_GetClassName(instance, &name) != MI_RESULT_OK ||
      MI_Instance_GetNameSpace(instance, &space) != MI_RESULT_OK) return false;
  *net_adapter = Same(name, L"MSFT_NetAdapter") &&
      (Same(space, L"root/StandardCimv2") || Same(space, L"root\\StandardCimv2"));
  return *net_adapter ||
      ((Same(name, L"Win32_NetworkAdapter") ||
        Same(name, L"Win32_NetworkAdapterConfiguration")) &&
       (Same(space, L"root/cimv2") || Same(space, L"root\\cimv2")));
}

static bool SixByteMac(const MI_Char* value) {
  if (!value) return false;
  unsigned digits = 0;
  for (; *value; ++value) {
    if ((*value >= L'0' && *value <= L'9') ||
        (*value >= L'a' && *value <= L'f') ||
        (*value >= L'A' && *value <= L'F')) ++digits;
    else if (*value != L':' && *value != L'-') return false;
  }
  return digits == 12;
}

static MI_Char* MacView(const MI_Char* original) {
  if (wcschr(original, L':')) return g_colon;
  if (wcschr(original, L'-')) return g_dash;
  return g_plain;
}

static bool Replace(const MI_Char* name, MI_Value* value, MI_Type type,
                    MI_Uint32 flags, bool net_adapter) {
  if (!name || !value || (flags & MI_FLAG_NULL)) return false;
  if (type == MI_STRING && SixByteMac(value->string) &&
      (Same(name, L"MACAddress") ||
       (net_adapter && Same(name, L"PermanentAddress")))) {
    value->string = MacView(value->string);
    return true;
  }
  // A borrowed MI_Value must not acquire per-call allocations or a mutable
  // thread-local buffer. Multi-address arrays are handled in the owned clone.
  if (net_adapter && Same(name, L"NetworkAddresses") && type == MI_STRINGA &&
      value->stringa.size == 1 && value->stringa.data &&
      SixByteMac(value->stringa.data[0])) {
    value->stringa.data = g_single_address;
    return true;
  }
  return false;
}

static MI_Result MI_CALL HookGetElement(const MI_Instance* self,
    const MI_Char* name, MI_Value* value, MI_Type* type, MI_Uint32* flags,
    MI_Uint32* index) {
  MI_Type actual_type = MI_BOOLEAN;
  MI_Uint32 actual_flags = 0;
  MI_Result result = TrueGetElement(self, name, value,
      type ? type : &actual_type, flags ? flags : &actual_flags, index);
  const DWORD error = GetLastError();
  bool net_adapter = false;
  if (result == MI_RESULT_OK && value && LocalSource(self) &&
      NetworkInstance(self, &net_adapter) &&
      Replace(name, value, type ? *type : actual_type,
              flags ? *flags : actual_flags, net_adapter))
    EnvBoxAuditEvent("MI.Instance.GetElement", 1, "mac-virtual-view");
  SetLastError(error);
  return result;
}

static MI_Result MI_CALL HookGetElementAt(const MI_Instance* self,
    MI_Uint32 index, const MI_Char** name, MI_Value* value, MI_Type* type,
    MI_Uint32* flags) {
  const MI_Char* actual_name = nullptr;
  MI_Type actual_type = MI_BOOLEAN;
  MI_Uint32 actual_flags = 0;
  MI_Result result = TrueGetElementAt(self, index, name ? name : &actual_name,
      value, type ? type : &actual_type, flags ? flags : &actual_flags);
  const DWORD error = GetLastError();
  bool net_adapter = false;
  if (result == MI_RESULT_OK && value && LocalSource(self) &&
      NetworkInstance(self, &net_adapter) &&
      Replace(name ? *name : actual_name, value, type ? *type : actual_type,
              flags ? *flags : actual_flags, net_adapter))
    EnvBoxAuditEvent("MI.Instance.GetElementAt", 1, "mac-virtual-view");
  SetLastError(error);
  return result;
}

static MI_Result MI_CALL HookClone(const MI_Instance* self, MI_Instance** clone) {
  MI_Result result = TrueClone(self, clone);
  const DWORD error = GetLastError();
  bool net_adapter = false;
  if (result == MI_RESULT_OK && clone && *clone && LocalSource(self) &&
      NetworkInstance(*clone, &net_adapter)) {
    bool rewritten = false;
    MI_Uint32 count = 0;
    result = MI_Instance_GetElementCount(*clone, &count);
    for (MI_Uint32 i = 0; result == MI_RESULT_OK && i < count; ++i) {
      const MI_Char* name = nullptr;
      MI_Value value = {};
      MI_Type type = MI_BOOLEAN;
      MI_Uint32 flags = 0;
      result = TrueGetElementAt(*clone, i, &name, &value, &type, &flags);
      if (result != MI_RESULT_OK) break;
      bool changed = Replace(name, &value, type, flags, net_adapter);
      // SetElement copies the strings into this owned client snapshot; no
      // provider update or change to the original borrowed instance occurs.
      if (!changed && net_adapter && Same(name, L"NetworkAddresses") &&
          type == MI_STRINGA && !(flags & MI_FLAG_NULL) &&
          value.stringa.data && value.stringa.size > 1) {
        try {
          std::vector<MI_Char*> addresses(value.stringa.data,
              value.stringa.data + value.stringa.size);
          for (auto& address : addresses) {
            if (SixByteMac(address)) { address = g_plain; changed = true; }
          }
          if (changed) {
            value.stringa.data = addresses.data();
            result = MI_Instance_SetElement(*clone, name, &value, type, 0);
            rewritten = rewritten || result == MI_RESULT_OK;
          }
        } catch (...) { result = MI_RESULT_FAILED; }
      } else if (changed) {
        result = MI_Instance_SetElement(*clone, name, &value, type, 0);
        rewritten = rewritten || result == MI_RESULT_OK;
      }
    }
    if (result != MI_RESULT_OK) {
      MI_Instance_Delete(*clone);
      *clone = nullptr;
      EnvBoxAuditEvent("MI.Instance.Clone", 0, "mac-clone-rewrite-failed");
    } else EnvBoxAuditEvent("MI.Instance.Clone", rewritten ? 1 : 0,
        rewritten ? "mac-client-snapshot" : "mac-no-six-byte-address");
  }
  SetLastError(error);
  return result;
}

static void BindOperation(OperationSource* source, MI_Operation* operation,
                          unsigned long long generation) {
  if (!operation || !operation->ft) return;
  SourceLock lock;
  if (source->active && source->generation == generation) source->key = Key(operation);
}

template<auto Member, class... Args>
static void MI_CALL ForwardCallback(MI_Operation* operation, void* context,
                                   Args... args) {
  auto* source = static_cast<OperationSource*>(context);
  CallbackLease lifetime(source);
  const auto original = lifetime.original;
  (original.*Member)(operation, original.callbackContext, args...);
}

static void MI_CALL InstanceCallback(MI_Operation* operation, void* context,
    const MI_Instance* instance, MI_Boolean more, MI_Result result,
    const MI_Char* error, const MI_Instance* details,
    MI_Result (MI_CALL* acknowledgement)(MI_Operation*));

static MI_Result MI_CALL AcknowledgeInstance(MI_Operation* operation) {
  OperationSource* source = nullptr;
  MI_Result (MI_CALL* native)(MI_Operation*) = nullptr;
  unsigned long long epoch = 0;
  { SourceLock lock;
    for (auto& item : g_operations)
      if (item.active && item.key == Key(operation) && item.acknowledgement_pending) {
        source = &item; native = item.native_acknowledgement;
        epoch = item.acknowledgement_epoch;
        ++item.calls_running;
        item.borrowed = nullptr;
        break;
      }
  }
  if (!source || !native) return MI_RESULT_INVALID_PARAMETER;
  MI_Result result = native(operation);
  const DWORD error = GetLastError();
  { SourceLock lock;
    --source->calls_running;
    // Native ack can deliver the next result before returning; don't clear
    // that newer result's pending acknowledgement or borrowed instance.
    if (result == MI_RESULT_OK && source->acknowledgement_epoch == epoch)
      source->acknowledgement_pending = false;
    RecycleOperation(*source);
  }
  SetLastError(error);
  return result;
}

static void MI_CALL InstanceCallback(MI_Operation* operation, void* context,
    const MI_Instance* instance, MI_Boolean more, MI_Result result,
    const MI_Char* error, const MI_Instance* details,
    MI_Result (MI_CALL* acknowledgement)(MI_Operation*)) {
  auto* source = static_cast<OperationSource*>(context);
  CallbackLease lifetime(source);
  BindOperation(source, operation, lifetime.generation);
  { SourceLock lock;
    if (!more) source->completed = true;
    if (acknowledgement) {
      ++source->acknowledgement_epoch;
      source->native_acknowledgement = acknowledgement;
      source->acknowledgement_pending = true;
      source->borrowed = instance;
    }
  }
  const auto original = lifetime.original;
  CallbackSource trusted(instance);
  original.instanceResult(operation, original.callbackContext, instance, more,
      result, error, details, acknowledgement ? AcknowledgeInstance : nullptr);
}

static OperationSource* PrepareOperation(MI_OperationCallbacks* callbacks) {
  SourceLock lock;
  for (auto& source : g_operations) {
    if (source.active) continue;
    source.active = true;
    ++source.generation;
    source.key = {};
    source.borrowed = nullptr;
    source.asynchronous = callbacks && callbacks->instanceResult;
    source.completed = source.close_requested = false;
    source.acknowledgement_pending = false;
    source.native_acknowledgement = nullptr;
    source.callbacks_running = 0;
    source.calls_running = 1; // Retain context even for synchronous callbacks.
    source.original = callbacks ? *callbacks : MI_OperationCallbacks{};
    source.wrapped = source.original;
    source.wrapped.callbackContext = &source;
    auto& wrapped = source.wrapped;
    using Response = MI_Result (MI_CALL*)(MI_Operation*, MI_OperationCallback_ResponseType);
    using Ack = MI_Result (MI_CALL*)(MI_Operation*);
    if (wrapped.promptUser) wrapped.promptUser =
        ForwardCallback<&MI_OperationCallbacks::promptUser, const MI_Char*, MI_PromptType, Response>;
    if (wrapped.writeError) wrapped.writeError =
        ForwardCallback<&MI_OperationCallbacks::writeError, MI_Instance*, Response>;
    if (wrapped.writeMessage) wrapped.writeMessage =
        ForwardCallback<&MI_OperationCallbacks::writeMessage, MI_Uint32, const MI_Char*>;
    if (wrapped.writeProgress) wrapped.writeProgress =
        ForwardCallback<&MI_OperationCallbacks::writeProgress, const MI_Char*, const MI_Char*,
                        const MI_Char*, MI_Uint32, MI_Uint32>;
    if (wrapped.instanceResult) wrapped.instanceResult = InstanceCallback;
    if (wrapped.indicationResult) wrapped.indicationResult =
        ForwardCallback<&MI_OperationCallbacks::indicationResult, const MI_Instance*, const MI_Char*,
                        const MI_Char*, MI_Boolean, MI_Result, const MI_Char*, const MI_Instance*, Ack>;
    if (wrapped.classResult) wrapped.classResult =
        ForwardCallback<&MI_OperationCallbacks::classResult, const MI_Class*, MI_Boolean,
                        MI_Result, const MI_Char*, const MI_Instance*, Ack>;
    if (wrapped.streamedParameterResult) wrapped.streamedParameterResult =
        ForwardCallback<&MI_OperationCallbacks::streamedParameterResult, const MI_Char*,
                        MI_Type, const MI_Value*, Ack>;
    return &source;
  }
  return nullptr;
}

static MI_Result MI_CALL FailedClose(MI_Operation* operation) {
  if (operation) *operation = {};
  return MI_RESULT_OK;
}
static MI_Result MI_CALL FailedCancel(MI_Operation*, MI_CancellationReason) {
  return MI_RESULT_OK;
}
static MI_Result MI_CALL FailedGetSession(MI_Operation*, MI_Session* session) {
  if (session) *session = {};
  return MI_RESULT_FAILED;
}
static MI_Result MI_CALL FailedGet(MI_Operation*, const MI_Instance** instance,
    MI_Boolean* more, MI_Result* result, const MI_Char** error,
    const MI_Instance** details) {
  if (instance) *instance = nullptr;
  if (more) *more = MI_FALSE;
  if (result) *result = MI_RESULT_SERVER_LIMITS_EXCEEDED;
  if (error) *error = L"EnvBox CIM MAC operation limit exceeded";
  if (details) *details = nullptr;
  return MI_RESULT_OK;
}
static MI_Result MI_CALL FailedIndication(MI_Operation* operation,
    const MI_Instance** instance, const MI_Char** bookmark, const MI_Char** machine,
    MI_Boolean* more, MI_Result* result, const MI_Char** error,
    const MI_Instance** details) {
  if (bookmark) *bookmark = nullptr;
  if (machine) *machine = nullptr;
  return FailedGet(operation, instance, more, result, error, details);
}
static MI_Result MI_CALL FailedClass(MI_Operation* operation, const MI_Class** klass,
    MI_Boolean* more, MI_Result* result, const MI_Char** error,
    const MI_Instance** details) {
  if (klass) *klass = nullptr;
  return FailedGet(operation, nullptr, more, result, error, details);
}
static const MI_OperationFT g_failed_operation = {FailedClose, FailedCancel,
    FailedGetSession, FailedGet, FailedIndication, FailedClass};
static void FailOperation(MI_Operation* operation, MI_OperationCallbacks* callbacks) {
  if (operation) { *operation = {}; operation->ft = &g_failed_operation; }
  EnvBoxAuditEvent("MI.Session.Operation", 0, "mac-operation-limit-exceeded");
  if (callbacks && callbacks->instanceResult)
    callbacks->instanceResult(operation, callbacks->callbackContext, nullptr,
        MI_FALSE, MI_RESULT_SERVER_LIMITS_EXCEEDED,
        L"EnvBox CIM MAC operation limit exceeded", nullptr, nullptr);
}

template<class Invoke>
static void ReadOperation(MI_Session* session, MI_OperationCallbacks* callbacks,
                          MI_Operation* operation, Invoke invoke) {
  if (!LocalSession(session)) { invoke(callbacks); return; }
  OperationSource* source = PrepareOperation(callbacks);
  if (!source) { FailOperation(operation, callbacks); return; }
  const auto generation = source->generation;
  invoke(callbacks ? &source->wrapped : nullptr);
  const DWORD error = GetLastError();
  BindOperation(source, operation, generation);
  {
    SourceLock lock;
    --source->calls_running;
    if (!operation || !operation->ft) source->close_requested = true;
    RecycleOperation(*source);
  }
  SetLastError(error);
}

static void MI_CALL HookSessionGet(MI_Session* session, MI_Uint32 flags,
    MI_OperationOptions* options, const MI_Char* space, const MI_Instance* inbound,
    MI_OperationCallbacks* callbacks, MI_Operation* operation) {
  ReadOperation(session, callbacks, operation, [&](MI_OperationCallbacks* wrapped) {
    TrueSessionGet(session, flags, options, space, inbound, wrapped, operation);
  });
}
static void MI_CALL HookEnumerate(MI_Session* session, MI_Uint32 flags,
    MI_OperationOptions* options, const MI_Char* space, const MI_Char* klass,
    MI_Boolean keys_only, MI_OperationCallbacks* callbacks, MI_Operation* operation) {
  ReadOperation(session, callbacks, operation, [&](MI_OperationCallbacks* wrapped) {
    TrueEnumerate(session, flags, options, space, klass, keys_only, wrapped, operation);
  });
}
static void MI_CALL HookQuery(MI_Session* session, MI_Uint32 flags,
    MI_OperationOptions* options, const MI_Char* space, const MI_Char* dialect,
    const MI_Char* expression, MI_OperationCallbacks* callbacks, MI_Operation* operation) {
  ReadOperation(session, callbacks, operation, [&](MI_OperationCallbacks* wrapped) {
    TrueQuery(session, flags, options, space, dialect, expression, wrapped, operation);
  });
}
static void MI_CALL HookAssociator(MI_Session* session, MI_Uint32 flags,
    MI_OperationOptions* options, const MI_Char* space, const MI_Instance* inbound,
    const MI_Char* assoc, const MI_Char* klass, const MI_Char* role,
    const MI_Char* result_role, MI_Boolean keys_only,
    MI_OperationCallbacks* callbacks, MI_Operation* operation) {
  ReadOperation(session, callbacks, operation, [&](MI_OperationCallbacks* wrapped) {
    TrueAssociator(session, flags, options, space, inbound, assoc, klass, role,
                   result_role, keys_only, wrapped, operation);
  });
}
static void MI_CALL HookReference(MI_Session* session, MI_Uint32 flags,
    MI_OperationOptions* options, const MI_Char* space, const MI_Instance* inbound,
    const MI_Char* klass, const MI_Char* role, MI_Boolean keys_only,
    MI_OperationCallbacks* callbacks, MI_Operation* operation) {
  ReadOperation(session, callbacks, operation, [&](MI_OperationCallbacks* wrapped) {
    TrueReference(session, flags, options, space, inbound, klass, role,
                  keys_only, wrapped, operation);
  });
}

static MI_Result MI_CALL HookNewSession(MI_Application* application,
    const MI_Char* protocol, const MI_Char* destination, MI_DestinationOptions* options,
    MI_SessionCallbacks* callbacks, MI_Instance** extended_error, MI_Session* session) {
  bool local = !destination || Same(destination, L".") || Same(destination, L"localhost");
  for (const auto& host : g_host) local = local || (host[0] && Same(destination, host));
  if (!local) return TrueNewSession(application, protocol, destination, options,
                                  callbacks, extended_error, session);
  SessionSource* source = nullptr;
  { SourceLock lock;
    for (auto& item : g_sessions) if (!item.active) {
      item.active = true; item.key = {}; source = &item; break;
    }
  }
  if (!source) {
    if (session) *session = {};
    if (extended_error) *extended_error = nullptr;
    EnvBoxAuditEvent("MI.Application.NewSession", 0, "mac-session-limit-exceeded");
    return MI_RESULT_SERVER_LIMITS_EXCEEDED;
  }
  MI_Result result = TrueNewSession(application, protocol, destination, options,
                                    callbacks, extended_error, session);
  const DWORD error = GetLastError();
  { SourceLock lock;
    if (result == MI_RESULT_OK && session) source->key = Key(session);
    else source->active = false;
  }
  SetLastError(error);
  return result;
}
static MI_Result MI_CALL HookSessionClose(MI_Session* session, void* context,
                                         void (MI_CALL* callback)(void*)) {
  const Key key(session);
  MI_Result result = TrueSessionClose(session, context, callback);
  const DWORD error = GetLastError();
  if (result == MI_RESULT_OK) {
    SourceLock lock;
    for (auto& source : g_sessions)
      if (source.active && source.key == key) source.active = false;
  }
  SetLastError(error);
  return result;
}
static MI_Result MI_CALL HookOperationGet(MI_Operation* operation,
    const MI_Instance** instance, MI_Boolean* more, MI_Result* result,
    const MI_Char** error, const MI_Instance** details) {
  OperationSource* source = nullptr;
  unsigned long long generation = 0;
  { SourceLock lock;
    for (auto& item : g_operations) if (item.active && item.key == Key(operation)) {
      source = &item; generation = item.generation; item.borrowed = nullptr; break;
    }
  }
  MI_Result status = TrueOperationGet(operation, instance, more, result, error, details);
  const DWORD last_error = GetLastError();
  if (source && status == MI_RESULT_OK && instance) {
    SourceLock lock;
    if (source->active && source->generation == generation) source->borrowed = *instance;
  }
  SetLastError(last_error);
  return status;
}
static MI_Result MI_CALL HookOperationClose(MI_Operation* operation) {
  const Key key(operation);
  OperationSource* source = nullptr;
  { SourceLock lock;
    for (auto& item : g_operations) if (item.active && item.key == key) {
      source = &item; ++source->calls_running; break;
    }
  }
  MI_Result result = TrueOperationClose(operation);
  const DWORD error = GetLastError();
  // MI closes asynchronous operations asynchronously. Retain their context
  // through the final callback and any callback currently executing.
  if (source) {
    SourceLock lock;
    --source->calls_running;
    if (result == MI_RESULT_OK) source->close_requested = true;
    RecycleOperation(*source);
  }
  SetLastError(error);
  return result;
}

static BOOL CALLBACK InstallReadSeams(PINIT_ONCE, PVOID context, PVOID*) {
  auto* application = static_cast<MI_Application*>(context);
  MI_Instance* instance = nullptr;
  if (MI_Application_NewInstance(application, L"EnvBoxCimReadSeam", nullptr,
                                 &instance) != MI_RESULT_OK || !instance)
    return TRUE;
  TrueGetElement = instance->ft->GetElement;
  TrueGetElementAt = instance->ft->GetElementAt;
  TrueClone = instance->ft->Clone;
  MI_Instance_Delete(instance);
  TrueNewSession = application->ft->NewSession;
  const auto* session_ft = mi_clientFT->sessionFT;
  const auto* operation_ft = mi_clientFT->operationFT;
  TrueSessionClose = session_ft->Close;
  TrueSessionGet = session_ft->GetInstance;
  TrueEnumerate = session_ft->EnumerateInstances;
  TrueQuery = session_ft->QueryInstances;
  TrueAssociator = session_ft->AssociatorInstances;
  TrueReference = session_ft->ReferenceInstances;
  TrueOperationGet = operation_ft->GetInstance;
  TrueOperationClose = operation_ft->Close;
  LONG status = NO_ERROR;
  // Late initialization can run concurrently with unrelated MI users. Enlist
  // all accessible process threads so Detours adjusts instruction pointers.
  std::vector<OwnedHandle> threads;
  OwnedHandle snapshot(CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0));
  if (snapshot.get() == INVALID_HANDLE_VALUE) status = GetLastError();
  else {
    THREADENTRY32 entry = {}; entry.dwSize = sizeof(entry);
    if (!Thread32First(snapshot.get(), &entry)) status = GetLastError();
    else do {
      if (entry.th32OwnerProcessID != GetCurrentProcessId() ||
          entry.th32ThreadID == GetCurrentThreadId()) continue;
      OwnedHandle thread(OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT |
          THREAD_SET_CONTEXT | THREAD_QUERY_INFORMATION, FALSE, entry.th32ThreadID));
      if (!thread) {
        if (GetLastError() != ERROR_INVALID_PARAMETER) status = GetLastError();
        continue;
      }
      try { threads.push_back(std::move(thread)); }
      catch (...) { return TRUE; }
    } while (Thread32Next(snapshot.get(), &entry));
  }
  if (status != NO_ERROR) return TRUE;
  status = DetourTransactionBegin();
  if (status != NO_ERROR) return TRUE;
  DetourTransactionGuard transaction;
  status = DetourUpdateThread(GetCurrentThread());
  for (const auto& thread : threads) {
    LONG update = DetourUpdateThread(thread.get());
    if (update != NO_ERROR) status = update;
  }
  if (status == NO_ERROR && EnvBoxAttach(&TrueGetElement, HookGetElement) &&
      EnvBoxAttach(&TrueGetElementAt, HookGetElementAt) &&
      EnvBoxAttach(&TrueClone, HookClone) &&
      EnvBoxAttach(&TrueNewSession, HookNewSession) &&
      EnvBoxAttach(&TrueSessionClose, HookSessionClose) &&
      EnvBoxAttach(&TrueSessionGet, HookSessionGet) &&
      EnvBoxAttach(&TrueEnumerate, HookEnumerate) &&
      EnvBoxAttach(&TrueQuery, HookQuery) &&
      EnvBoxAttach(&TrueAssociator, HookAssociator) &&
      EnvBoxAttach(&TrueReference, HookReference) &&
      EnvBoxAttach(&TrueOperationGet, HookOperationGet) &&
      EnvBoxAttach(&TrueOperationClose, HookOperationClose))
  {
    g_ready = DetourTransactionCommit() == NO_ERROR;
    transaction.active = false;
  }
  return TRUE; // Latch failure too; never advertise host MAC as a valid result.
}

static MI_Result MI_MAIN_CALL HookInitialize(MI_Uint32 flags,
    const MI_Char* id, MI_Instance** extended_error, MI_Application* application) {
  MI_Result result = TrueInitialize(flags, id, extended_error, application);
  const DWORD error = GetLastError();
  if (result == MI_RESULT_OK && application) {
    InitOnceExecuteOnce(&g_once, InstallReadSeams, application, nullptr);
    if (!g_ready) {
      MI_Application_Close(application);
      *application = {};
      EnvBoxAuditEvent("MI.Application.Initialize", 0, "mac-read-seams-failed");
      result = MI_RESULT_FAILED;
    } else EnvBoxAuditEvent("MI.Application.Initialize", 1, "mac-read-seams-ready=12");
  }
  SetLastError(error);
  return result;
}

int EnvBoxInstallCimIdentityHooks() {
  const auto* profile = EnvBoxProfile();
  if (!profile->identity_mac_address[0]) return 0;
  const COMPUTER_NAME_FORMAT formats[] = {ComputerNameNetBIOS,
      ComputerNameDnsHostname, ComputerNameDnsFullyQualified};
  for (unsigned i = 0; i < 3; ++i) {
    DWORD size = static_cast<DWORD>(_countof(g_host[i]));
    if (!GetComputerNameExW(formats[i], g_host[i], &size)) g_host[i][0] = 0;
  }
  constexpr wchar_t hex[] = L"0123456789ABCDEF";
  for (unsigned i = 0; i < 6; ++i) {
    const BYTE byte = profile->identity_mac_bytes[i];
    g_plain[2*i] = g_colon[3*i] = g_dash[3*i] = hex[byte >> 4];
    g_plain[2*i+1] = g_colon[3*i+1] = g_dash[3*i+1] = hex[byte & 15];
    if (i != 5) { g_colon[3*i+2] = L':'; g_dash[3*i+2] = L'-'; }
  }
  return EnvBoxAttach(&TrueInitialize, HookInitialize);
}
