// WinRT Calendar's default time zone bypasses the Win32 time-zone APIs. Set
// the Profile zone once at activation; later explicit ChangeTimeZone calls
// remain the caller's choice.

#include "hooks.h"

#include <wchar.h>
#include <roapi.h>
#include <windows.globalization.h>
#include <winstring.h>
#include <wrl.h>

#include "audit.h"

using CalendarTimeZone = ABI::Windows::Globalization::ITimeZoneOnCalendar;

static HRESULT(WINAPI* TrueRoGetActivationFactory)(HSTRING, REFIID, void**) =
    RoGetActivationFactory;
static HRESULT(WINAPI* TrueRoActivateInstance)(HSTRING, IInspectable**) =
    RoActivateInstance;

static bool IsCalendar(HSTRING class_id) {
  if (class_id == nullptr) {
    return false;
  }
  UINT32 length = 0;
  const wchar_t* name = WindowsGetStringRawBuffer(class_id, &length);
  constexpr wchar_t kCalendar[] = L"Windows.Globalization.Calendar";
  return length == (sizeof(kCalendar) / sizeof(wchar_t)) - 1 &&
         wmemcmp(name, kCalendar, length) == 0;
}

static void ApplyProfileZone(IInspectable* instance) {
  const RuntimeProfile* profile = EnvBoxProfile();
  if (instance == nullptr || profile == nullptr || !profile->has_tz ||
      profile->tz_iana[0] == L'\0') {
    return;
  }

  CalendarTimeZone* calendar_zone = nullptr;
  HRESULT hr = instance->QueryInterface(__uuidof(CalendarTimeZone),
                                        reinterpret_cast<void**>(&calendar_zone));
  if (FAILED(hr)) {
    return;  // The activation method may also serve unrelated WinRT classes.
  }
  // The Calendar call consumes the HSTRING synchronously. A reference string
  // avoids a heap allocation and WindowsDeleteString on every activation.
  HSTRING_HEADER zone_header = {};
  HSTRING zone_id = nullptr;
  hr = WindowsCreateStringReference(
      profile->tz_iana, static_cast<UINT32>(wcslen(profile->tz_iana)),
      &zone_header, &zone_id);
  if (SUCCEEDED(hr)) {
    hr = calendar_zone->ChangeTimeZone(zone_id);
  }
  calendar_zone->Release();
  EnvBoxAuditEventW("WinRT.Calendar.ChangeTimeZone", SUCCEEDED(hr),
                    profile->tz_iana);
}

// RoGetActivationFactory returns a COM interface, so wrapping only the
// Calendar factory avoids changing a process-wide COM vtable. The proxy owns
// the original activation factory but implements every public Calendar factory
// interface itself. This is important for COM identity: returning an inner
// pointer from QueryInterface would let callers bypass the Profile hook via
// IUnknown, IInspectable, IAgileObject, or ICalendarFactory2.
class CalendarActivationFactoryProxy final
    : public Microsoft::WRL::RuntimeClass<
          Microsoft::WRL::RuntimeClassFlags<
              Microsoft::WRL::WinRtClassicComMix>,
          IActivationFactory, ABI::Windows::Globalization::ICalendarFactory,
          ABI::Windows::Globalization::ICalendarFactory2,
          Microsoft::WRL::CloakedIid<IAgileObject>> {
 public:
  explicit CalendarActivationFactoryProxy(IActivationFactory* inner) {
    inner_.Attach(inner);
    if (inner_.Get() != nullptr) {
      inner_->QueryInterface(
          __uuidof(ABI::Windows::Globalization::ICalendarFactory),
          reinterpret_cast<void**>(calendar_factory_.GetAddressOf()));
      inner_->QueryInterface(
          __uuidof(ABI::Windows::Globalization::ICalendarFactory2),
          reinterpret_cast<void**>(calendar_factory2_.GetAddressOf()));
    }
  }

  HRESULT STDMETHODCALLTYPE GetRuntimeClassName(HSTRING* class_name) override {
    return inner_ == nullptr ? E_UNEXPECTED
                             : inner_->GetRuntimeClassName(class_name);
  }

  HRESULT STDMETHODCALLTYPE GetTrustLevel(TrustLevel* trust_level) override {
    return inner_ == nullptr ? E_UNEXPECTED
                             : inner_->GetTrustLevel(trust_level);
  }

  HRESULT STDMETHODCALLTYPE ActivateInstance(IInspectable** instance) override {
    // The factory's timezone-at-construction method requires a non-null
    // language iterable. The default activation contract does not provide one,
    // so keep the original activation and apply the immutable Profile zone.
    // This is also the fail-open path for older Windows builds.
    HRESULT hr =
        inner_ == nullptr ? E_UNEXPECTED : inner_->ActivateInstance(instance);
    const DWORD saved_error = GetLastError();
    if (SUCCEEDED(hr) && instance != nullptr) {
      ApplyProfileZone(*instance);
    }
    SetLastError(saved_error);
    return hr;
  }

  HRESULT STDMETHODCALLTYPE CreateCalendarDefaultCalendarAndClock(
      __FIIterable_1_HSTRING* languages,
      ABI::Windows::Globalization::ICalendar** result) override {
    if (calendar_factory_ == nullptr) {
      return E_NOINTERFACE;
    }
    HRESULT hr = calendar_factory_->CreateCalendarDefaultCalendarAndClock(
        languages, result);
    const DWORD saved_error = GetLastError();
    if (SUCCEEDED(hr) && result != nullptr && *result != nullptr) {
      ApplyProfileZone(reinterpret_cast<IInspectable*>(*result));
    }
    SetLastError(saved_error);
    return hr;
  }

  HRESULT STDMETHODCALLTYPE CreateCalendar(
      __FIIterable_1_HSTRING* languages, HSTRING calendar, HSTRING clock,
      ABI::Windows::Globalization::ICalendar** result) override {
    if (calendar_factory_ == nullptr) {
      return E_NOINTERFACE;
    }
    HRESULT hr =
        calendar_factory_->CreateCalendar(languages, calendar, clock, result);
    const DWORD saved_error = GetLastError();
    if (SUCCEEDED(hr) && result != nullptr && *result != nullptr) {
      ApplyProfileZone(reinterpret_cast<IInspectable*>(*result));
    }
    SetLastError(saved_error);
    return hr;
  }

  HRESULT STDMETHODCALLTYPE CreateCalendarWithTimeZone(
      __FIIterable_1_HSTRING* languages, HSTRING calendar, HSTRING clock,
      HSTRING time_zone_id,
      ABI::Windows::Globalization::ICalendar** result) override {
    if (calendar_factory2_ == nullptr) {
      return E_NOINTERFACE;
    }
    // This is an explicit caller-selected timezone. Preserve it exactly.
    HRESULT hr = calendar_factory2_->CreateCalendarWithTimeZone(
        languages, calendar, clock, time_zone_id, result);
    const DWORD saved_error = GetLastError();
    SetLastError(saved_error);
    return hr;
  }

 private:
  Microsoft::WRL::ComPtr<IActivationFactory> inner_;
  Microsoft::WRL::ComPtr<ABI::Windows::Globalization::ICalendarFactory>
      calendar_factory_;
  Microsoft::WRL::ComPtr<ABI::Windows::Globalization::ICalendarFactory2>
      calendar_factory2_;
};

// RoGetActivationFactory normally returns the same Calendar factory identity
// for repeated calls. Keep one proxy for that identity as well, so callers
// comparing IUnknown pointers across separate factory lookups see the same
// COM object. The lock only covers the low-frequency factory lookup path;
// Calendar methods never touch this cache.
static SRWLOCK g_calendar_factory_cache_lock = SRWLOCK_INIT;
static Microsoft::WRL::ComPtr<IUnknown> g_calendar_factory_identity;
static Microsoft::WRL::ComPtr<CalendarActivationFactoryProxy>
    g_calendar_factory_proxy;

// Takes ownership of inner. The returned ComPtr is an additional caller
// reference; the cache keeps its own reference for later RoGetActivationFactory
// calls.
static Microsoft::WRL::ComPtr<CalendarActivationFactoryProxy>
GetOrCreateCalendarFactoryProxy(IActivationFactory* inner) {
  Microsoft::WRL::ComPtr<CalendarActivationFactoryProxy> proxy;
  if (inner == nullptr) {
    return proxy;
  }

  Microsoft::WRL::ComPtr<IUnknown> identity;
  if (FAILED(inner->QueryInterface(IID_IUnknown,
                                   reinterpret_cast<void**>(
                                       identity.GetAddressOf())))) {
    inner->Release();
    return proxy;
  }

  AcquireSRWLockShared(&g_calendar_factory_cache_lock);
  if (g_calendar_factory_identity.Get() == identity.Get() &&
      g_calendar_factory_proxy.Get() != nullptr) {
    proxy = g_calendar_factory_proxy;
    ReleaseSRWLockShared(&g_calendar_factory_cache_lock);
    inner->Release();
    return proxy;
  }
  ReleaseSRWLockShared(&g_calendar_factory_cache_lock);

  auto candidate = Microsoft::WRL::Make<CalendarActivationFactoryProxy>(inner);
  if (candidate == nullptr) {
    inner->Release();
    return proxy;
  }

  AcquireSRWLockExclusive(&g_calendar_factory_cache_lock);
  if (g_calendar_factory_identity.Get() == identity.Get() &&
      g_calendar_factory_proxy.Get() != nullptr) {
    proxy = g_calendar_factory_proxy;
  } else {
    g_calendar_factory_identity = identity;
    g_calendar_factory_proxy = candidate;
    proxy = candidate;
  }
  ReleaseSRWLockExclusive(&g_calendar_factory_cache_lock);
  return proxy;
}

static bool IsCalendarFactoryInterface(REFIID iid) {
  return iid == IID_IUnknown || iid == IID_IInspectable ||
         iid == IID_IActivationFactory || iid == IID_IAgileObject ||
         iid == IID_IMarshal ||
         iid == __uuidof(ABI::Windows::Globalization::ICalendarFactory) ||
         iid == __uuidof(ABI::Windows::Globalization::ICalendarFactory2);
}

static void WrapCalendarFactory(REFIID requested_iid, void** factory) {
  if (!IsCalendarFactoryInterface(requested_iid) || factory == nullptr ||
      *factory == nullptr) {
    return;
  }

  auto* returned = static_cast<IUnknown*>(*factory);
  IActivationFactory* inner = nullptr;
  if (FAILED(returned->QueryInterface(
          __uuidof(IActivationFactory), reinterpret_cast<void**>(&inner)))) {
    return;  // Fail Open: preserve the original COM result.
  }

  // The proxy advertises both Calendar factory interfaces. Do not create it
  // for an older or alternate implementation that lacks either one; returning
  // such a proxy would make QI report an interface the inner object cannot
  // actually service.
  ABI::Windows::Globalization::ICalendarFactory* calendar_factory = nullptr;
  ABI::Windows::Globalization::ICalendarFactory2* calendar_factory2 = nullptr;
  const HRESULT factory_hr = returned->QueryInterface(
      __uuidof(ABI::Windows::Globalization::ICalendarFactory),
      reinterpret_cast<void**>(&calendar_factory));
  const HRESULT factory2_hr = returned->QueryInterface(
      __uuidof(ABI::Windows::Globalization::ICalendarFactory2),
      reinterpret_cast<void**>(&calendar_factory2));
  if (FAILED(factory_hr) || FAILED(factory2_hr)) {
    if (calendar_factory != nullptr) {
      calendar_factory->Release();
    }
    if (calendar_factory2 != nullptr) {
      calendar_factory2->Release();
    }
    inner->Release();
    return;  // Fail Open: preserve the original COM result.
  }
  calendar_factory->Release();
  calendar_factory2->Release();

  auto proxy = GetOrCreateCalendarFactoryProxy(inner);
  if (proxy == nullptr) {
    return;  // Fail Open: preserve the original COM result.
  }

  void* replacement = nullptr;
  HRESULT hr = proxy->QueryInterface(requested_iid, &replacement);
  if (SUCCEEDED(hr) && replacement != nullptr) {
    // Release the interface reference returned by the original API only after
    // a proxy reference has been obtained for the caller.
    returned->Release();
    *factory = replacement;
  }
}

static HRESULT WINAPI HookRoGetActivationFactory(HSTRING class_id, REFIID iid,
                                                  void** factory) {
  HRESULT hr = TrueRoGetActivationFactory(class_id, iid, factory);
  const DWORD saved_error = GetLastError();
  if (FAILED(hr) || factory == nullptr || *factory == nullptr ||
      !IsCalendar(class_id)) {
    SetLastError(saved_error);
    return hr;
  }

  WrapCalendarFactory(iid, factory);
  SetLastError(saved_error);
  return hr;
}

static HRESULT WINAPI HookRoActivateInstance(HSTRING class_id,
                                              IInspectable** instance) {
  HRESULT hr = TrueRoActivateInstance(class_id, instance);
  const DWORD saved_error = GetLastError();
  if (SUCCEEDED(hr) && instance != nullptr && IsCalendar(class_id)) {
    ApplyProfileZone(*instance);
  }
  SetLastError(saved_error);
  return hr;
}

int EnvBoxInstallWinRtTimeHooks() {
  const RuntimeProfile* profile = EnvBoxProfile();
  if (profile == nullptr || !profile->has_tz || profile->tz_iana[0] == L'\0') {
    return 0;
  }
  int ok = 0;
  ok += EnvBoxAttach(&TrueRoGetActivationFactory, HookRoGetActivationFactory);
  ok += EnvBoxAttach(&TrueRoActivateInstance, HookRoActivateInstance);
  return ok;
}
