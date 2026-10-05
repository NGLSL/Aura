use crate::{transport, Budget, CancelCallback, Error};
use std::{
    ffi::c_void,
    panic::{catch_unwind, AssertUnwindSafe},
    slice, str,
};

/// All buffers remain caller-owned. URL <=2047 bytes; literal IP <=63 bytes;
/// DNS query 12..65535 bytes; response capacity 12..65535. Buffers must not
/// overlap. This function never retains their pointers or the callback.
/// >0 response bytes; 0 failure; -1 cancellation. Error is a stable u32 enum.
/// Deadline is absolute Windows GetTickCount64 milliseconds. Port is taken
/// solely from URL authority, avoiding an inconsistent second port argument.
///
/// # Safety
/// Nonnull buffers must be valid for the supplied sizes for this entire call.
/// The optional callback uses C calling convention, must not unwind, and must
/// remain valid with its context until return. error must be writable if set.
#[no_mangle]
pub unsafe extern "C" fn envbox_doh_query(
    url: *const u8,
    url_length: usize,
    literal_ip: *const u8,
    ip_length: usize,
    query: *const u8,
    query_length: usize,
    response: *mut u8,
    capacity: usize,
    deadline: u64,
    cancelled: CancelCallback,
    context: *mut c_void,
    error: *mut u32,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if url.is_null()
            || literal_ip.is_null()
            || query.is_null()
            || response.is_null()
            || !(1..=2047).contains(&url_length)
            || !(1..=63).contains(&ip_length)
            || !(12..=65535).contains(&query_length)
            || !(12..=65535).contains(&capacity)
        {
            return Err(Error::Argument);
        }
        let url =
            str::from_utf8(slice::from_raw_parts(url, url_length)).map_err(|_| Error::Argument)?;
        let literal_ip = str::from_utf8(slice::from_raw_parts(literal_ip, ip_length))
            .map_err(|_| Error::Argument)?;
        let query = slice::from_raw_parts(query, query_length);
        let bytes = transport::query(
            url,
            literal_ip,
            query,
            Budget {
                deadline,
                cancelled,
                context,
            },
            None,
        )?;
        if bytes.len() > capacity {
            return Err(Error::BodyLimit);
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), response, bytes.len());
        Ok(bytes.len() as i32)
    }))
    .unwrap_or(Err(Error::Panic));
    match result {
        Ok(length) => {
            if !error.is_null() {
                *error = Error::None as u32;
            }
            length
        }
        Err(cause) => {
            if !error.is_null() {
                *error = cause as u32;
            }
            if cause == Error::Cancelled {
                -1
            } else {
                0
            }
        }
    }
}
