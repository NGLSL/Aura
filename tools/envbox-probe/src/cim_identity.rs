//! Read-only local CIM observations through the native MI client.
use crate::Field;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use windows::Win32::System::Wmi::*;

type CloseApplication = unsafe extern "system" fn(*mut MI_Application) -> MI_Result;
type CloseSession = unsafe extern "system" fn(
    *mut MI_Session,
    *mut c_void,
    Option<unsafe extern "system" fn(*mut c_void)>,
) -> MI_Result;
type CloseOperation = unsafe extern "system" fn(*mut MI_Operation) -> MI_Result;
type DeleteInstance = unsafe extern "system" fn(*mut MI_Instance) -> MI_Result;
type NewSession = unsafe extern "system" fn(
    *mut MI_Application,
    *const u16,
    *const u16,
    *mut MI_DestinationOptions,
    *mut MI_SessionCallbacks,
    *mut *mut MI_Instance,
    *mut MI_Session,
) -> MI_Result;
type Enumerate = unsafe extern "system" fn(
    *mut MI_Session,
    u32,
    *mut MI_OperationOptions,
    *const u16,
    *const u16,
    u8,
    *mut MI_OperationCallbacks,
    *mut MI_Operation,
);
type Query = unsafe extern "system" fn(
    *mut MI_Session,
    u32,
    *mut MI_OperationOptions,
    *const u16,
    *const u16,
    *const u16,
    *mut MI_OperationCallbacks,
    *mut MI_Operation,
);
type GetInstance = unsafe extern "system" fn(
    *mut MI_Operation,
    *mut *const MI_Instance,
    *mut u8,
    *mut MI_Result,
    *mut *const u16,
    *mut *const MI_Instance,
) -> MI_Result;
type GetElement = unsafe extern "system" fn(
    *const MI_Instance,
    *const u16,
    *mut MI_Value,
    *mut MI_Type,
    *mut u32,
    *mut u32,
) -> MI_Result;
type GetElementAt = unsafe extern "system" fn(
    *const MI_Instance,
    u32,
    *mut *const u16,
    *mut MI_Value,
    *mut MI_Type,
    *mut u32,
) -> MI_Result;
type CloneInstance =
    unsafe extern "system" fn(*const MI_Instance, *mut *mut MI_Instance) -> MI_Result;
type GetServerName = unsafe extern "system" fn(*const MI_Instance, *mut *const u16) -> MI_Result;

struct Application(MI_Application);
struct Session(MI_Session);
struct Operation(MI_Operation);
struct Instance(*mut MI_Instance);
impl Drop for Application {
    fn drop(&mut self) {
        unsafe {
            if !self.0.ft.is_null() {
                let close: CloseApplication = std::mem::transmute((*self.0.ft).Close);
                close(&mut self.0);
            }
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if !self.0.ft.is_null() {
                let close: CloseSession = std::mem::transmute((*self.0.ft).Close);
                close(&mut self.0, null_mut(), None);
            }
        }
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        unsafe {
            if !self.0.ft.is_null() {
                let close: CloseOperation = std::mem::transmute((*self.0.ft).Close);
                close(&mut self.0);
            }
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                let delete: DeleteInstance = std::mem::transmute((*(*self.0).ft).Delete);
                delete(self.0);
            }
        }
    }
}

fn push(fields: &mut Vec<Field>, name: impl Into<String>, value: impl Into<String>) {
    fields.push(Field {
        name: name.into(),
        value: value.into(),
    });
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
unsafe fn string(value: *const u16) -> String {
    if value.is_null() {
        return String::new();
    }
    let mut len = 0;
    while *value.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(value, len))
}
unsafe fn value_text(value: &MI_Value, kind: MI_Type, flags: u32) -> String {
    if flags & MI_FLAG_NULL != 0 {
        return "<null>".into();
    }
    match kind {
        MI_STRING => string(value.string),
        MI_STRINGA => {
            let a = value.stringa;
            if a.size == 0 {
                return String::new();
            }
            if a.data.is_null() {
                return "<invalid array>".into();
            }
            std::slice::from_raw_parts(a.data, a.size as usize)
                .iter()
                .map(|&v| string(v))
                .collect::<Vec<_>>()
                .join(",")
        }
        MI_UINT32 => value.uint32.to_string(),
        MI_UINT16 => value.uint16.to_string(),
        MI_BOOLEAN => (value.boolean != 0).to_string(),
        _ => format!("<type={}>", kind.0),
    }
}

unsafe fn property(
    fields: &mut Vec<Field>,
    prefix: &str,
    instance: *const MI_Instance,
    name: &str,
    by_index: bool,
) {
    let ft = &*(*instance).ft;
    let get: GetElement = std::mem::transmute(ft.GetElement);
    let mut value = MI_Value::default();
    let mut kind = MI_Type::default();
    let mut flags = 0;
    let mut index = 0;
    let name_w = wide(name);
    let status = get(
        instance,
        name_w.as_ptr(),
        &mut value,
        &mut kind,
        &mut flags,
        &mut index,
    );
    let text = if status.0 == 0 {
        value_text(&value, kind, flags)
    } else {
        String::new()
    };
    push(
        fields,
        format!("{prefix}_{name}"),
        format!(
            "status={};type={};flags={flags};value={text}",
            status.0, kind.0
        ),
    );
    if by_index && status.0 == 0 {
        let get_at: GetElementAt = std::mem::transmute(ft.GetElementAt);
        let mut actual_name = null();
        let status = get_at(
            instance,
            index,
            &mut actual_name,
            &mut value,
            &mut kind,
            &mut flags,
        );
        let text = if status.0 == 0 {
            value_text(&value, kind, flags)
        } else {
            String::new()
        };
        push(
            fields,
            format!("{prefix}_{name}_At"),
            format!(
                "status={};index={index};name={};type={};flags={flags};value={text}",
                status.0,
                string(actual_name),
                kind.0
            ),
        );
    }
}

unsafe fn observe(
    fields: &mut Vec<Field>,
    prefix: &str,
    instance: *const MI_Instance,
    properties: &[&str],
) {
    let get_server: GetServerName = std::mem::transmute((*(*instance).ft).GetServerName);
    let mut server = null();
    let status = get_server(instance, &mut server);
    push(
        fields,
        format!("{prefix}_ServerName"),
        format!("status={};value={}", status.0, string(server)),
    );
    for name in properties {
        property(fields, prefix, instance, name, true);
    }
    for name in ["Name", "InterfaceIndex"] {
        property(fields, prefix, instance, name, false);
    }
    let clone: CloneInstance = std::mem::transmute((*(*instance).ft).Clone);
    let mut owned = Instance(null_mut());
    let status = clone(instance, &mut owned.0);
    push(
        fields,
        format!("{prefix}_CloneStatus"),
        status.0.to_string(),
    );
    if status.0 == 0 && !owned.0.is_null() {
        for name in properties {
            property(fields, &format!("{prefix}_Clone"), owned.0, name, true);
        }
    }
}

unsafe fn read_class(
    fields: &mut Vec<Field>,
    session: &mut Session,
    namespace: &str,
    class: &str,
    properties: &[&str],
    query: bool,
) {
    let method = if query { "Query" } else { "Enumerate" };
    let prefix = format!("CIM_{class}_{method}");
    let namespace = wide(namespace);
    let mut operation = Operation(MI_Operation::default());
    if query {
        let read: Query = std::mem::transmute((*session.0.ft).QueryInstances);
        let dialect = wide("WQL");
        let expression = wide(&format!("SELECT * FROM {class}"));
        read(
            &mut session.0,
            0,
            null_mut(),
            namespace.as_ptr(),
            dialect.as_ptr(),
            expression.as_ptr(),
            null_mut(),
            &mut operation.0,
        );
    } else {
        let read: Enumerate = std::mem::transmute((*session.0.ft).EnumerateInstances);
        let class = wide(class);
        read(
            &mut session.0,
            0,
            null_mut(),
            namespace.as_ptr(),
            class.as_ptr(),
            0,
            null_mut(),
            &mut operation.0,
        );
    }
    if operation.0.ft.is_null() {
        push(fields, format!("{prefix}_Status"), "operation unavailable");
        return;
    }
    let get: GetInstance = std::mem::transmute((*operation.0.ft).GetInstance);
    let mut count = 0;
    loop {
        let mut instance = null();
        let mut more = 0;
        let mut result = MI_Result::default();
        let mut error = null();
        let status = get(
            &mut operation.0,
            &mut instance,
            &mut more,
            &mut result,
            &mut error,
            null_mut(),
        );
        if status.0 != 0 || result.0 != 0 {
            push(
                fields,
                format!("{prefix}_Status"),
                format!(
                    "status={};result={};error={};count={count}",
                    status.0,
                    result.0,
                    string(error)
                ),
            );
            break;
        }
        if !instance.is_null() {
            // Operation owns this instance until the next GetInstance or Close.
            observe(fields, &format!("{prefix}_{count}"), instance, properties);
            count += 1;
        }
        if more == 0 {
            push(
                fields,
                format!("{prefix}_Status"),
                format!("status=0;result=0;count={count}"),
            );
            break;
        }
    }
}

pub fn collect(fields: &mut Vec<Field>) {
    unsafe {
        let mut application = Application(MI_Application::default());
        let status = MI_Application_InitializeV1(0, None, None, &mut application.0);
        push(fields, "CIM_InitializeStatus", status.0.to_string());
        if status.0 != 0 || application.0.ft.is_null() {
            return;
        }
        let mut session = Session(MI_Session::default());
        let new_session: NewSession = std::mem::transmute((*application.0.ft).NewSession);
        let status = new_session(
            &mut application.0,
            null(),
            null(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut session.0,
        );
        push(fields, "CIM_SessionStatus", status.0.to_string());
        if status.0 != 0 || session.0.ft.is_null() {
            return;
        }
        for (namespace, class, properties) in [
            ("ROOT/cimv2", "Win32_NetworkAdapter", &["MACAddress"][..]),
            (
                "ROOT/cimv2",
                "Win32_NetworkAdapterConfiguration",
                &["MACAddress"][..],
            ),
            (
                "ROOT/StandardCimv2",
                "MSFT_NetAdapter",
                &["NetworkAddresses", "PermanentAddress"][..],
            ),
        ] {
            for query in [false, true] {
                read_class(fields, &mut session, namespace, class, properties, query);
            }
        }
    }
}
