use envbox_dns_doh::{trust::Snapshot, Budget, Error};
use std::{
    collections::HashMap,
    ffi::{c_void, CString},
    ptr,
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::GetLastError,
    System::{
        LibraryLoader::{GetProcAddress, LoadLibraryW},
        SystemInformation::GetTickCount64,
        Threading::{GetCurrentProcess, GetProcessHandleCount},
    },
};

unsafe extern "C" fn cancelled(context: *mut c_void) -> i32 {
    (!context.is_null() && GetTickCount64() >= *context.cast::<u64>()) as i32
}
fn materials(value: Option<&String>) -> Vec<Vec<u8>> {
    value
        .into_iter()
        .flat_map(|list| list.split(';'))
        .filter(|path| *path != "-" && !path.is_empty())
        .map(|path| std::fs::read(path).expect("fixture material"))
        .collect()
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert!(args.len() % 2 == 0, "all options are --name value pairs");
    let options: HashMap<_, _> = args
        .chunks_exact(2)
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect();
    let url = options.get("--url").expect("--url");
    let ip = options.get("--ip").expect("--ip");
    let uri: http::Uri = url.parse().expect("fixture URI");
    let port = uri
        .authority()
        .expect("authority")
        .port_u16()
        .unwrap_or(443);
    type SnapshotFn = unsafe extern "system" fn(*mut u8, u32) -> u32;
    let mut snapshot_fn: Option<SnapshotFn> = None;
    if let Some(path) = options.get("--trap") {
        unsafe {
            let path: Vec<u16> = path.encode_utf16().chain([0]).collect();
            let module = LoadLibraryW(path.as_ptr());
            assert!(
                !module.is_null(),
                "LoadLibrary trap error {}",
                GetLastError()
            );
            let allow: unsafe extern "system" fn(*const u8, u16) -> u32 = std::mem::transmute(
                GetProcAddress(module, b"DoHApiTrapAllowEndpoint\0".as_ptr())
                    .expect("trap AllowEndpoint"),
            );
            let install: unsafe extern "system" fn() -> u32 = std::mem::transmute(
                GetProcAddress(module, b"DoHApiTrapInstall\0".as_ptr()).expect("trap Install"),
            );
            snapshot_fn = Some(std::mem::transmute(
                GetProcAddress(module, b"DoHApiTrapSnapshot\0".as_ptr()).expect("trap Snapshot"),
            ));
            assert_eq!(
                allow(CString::new(ip.as_str()).unwrap().as_ptr().cast(), port),
                0
            );
            assert_eq!(install(), 0);
            // Intentionally retain this own-process hook module until exit.
        }
    }
    let repeats: usize = options
        .get("--repeat")
        .map(|value| value.parse().unwrap())
        .unwrap_or(1);
    assert!((1..=64).contains(&repeats));
    let packet = [
        0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 3, b'd', b'o', b'h', 7, b'f', b'i', b'x', b't',
        b'u', b'r', b'e', 4, b't', b'e', b's', b't', 0, 0, 65, 0, 1,
    ];
    let budget_ms: u64 = options
        .get("--budget-ms")
        .map(|value| value.parse().unwrap())
        .unwrap_or(2000);
    let mut baseline = 0;
    let mut final_handles = 0;
    let started = Instant::now();
    let mut outcome = Ok(Vec::new());
    for index in 0..repeats {
        let now = unsafe { GetTickCount64() };
        let mut cancel_at = options
            .get("--cancel-ms")
            .map(|value| now + value.parse::<u64>().unwrap())
            .unwrap_or(u64::MAX);
        let snapshot = Snapshot::fixture(
            materials(options.get("--roots")),
            materials(options.get("--ca")),
            materials(options.get("--crls")),
            materials(options.get("--deny")),
        )
        .expect("fixture trust shape");
        let budget = unsafe {
            Budget::from_callback(
                now + budget_ms,
                Some(cancelled),
                ptr::addr_of_mut!(cancel_at).cast(),
            )
        };
        outcome = envbox_dns_doh::query_fixture(url, ip, &packet, budget, snapshot);
        if index == 0 {
            unsafe {
                GetProcessHandleCount(GetCurrentProcess(), &mut baseline);
            }
        }
    }
    unsafe {
        GetProcessHandleCount(GetCurrentProcess(), &mut final_handles);
    }
    let (length, error) = match outcome {
        Ok(bytes) => (bytes.len(), Error::None),
        Err(error) => (0, error),
    };
    println!("result={length} error={} elapsed_ms={} handles_before={baseline} handles_after={final_handles}", error as u32, started.elapsed().as_millis());
    if let Some(snapshot) = snapshot_fn {
        let mut json = vec![0u8; 16384];
        assert_eq!(unsafe { snapshot(json.as_mut_ptr(), json.len() as u32) }, 0);
        let length = json
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(json.len());
        println!("trap={}", String::from_utf8_lossy(&json[..length]));
    }
    if length == 0 {
        std::process::exit(1);
    }
    assert!(
        final_handles <= baseline,
        "live handles increased after warmup"
    );
}
