//! App icon extraction + PNG cache (Kite `system/icons` simplified).
//! SHGetFileInfo → ExtractIconEx → HICON→RGBA→PNG under the config icon dir.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// In-memory id → cached PNG path (avoids re-extracting on every redraw).
fn mem_cache() -> &'static Mutex<HashMap<String, Option<PathBuf>>> {
    static C: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Extract/caches a 64×64 PNG for `icon_src` (`path` or `path,index`).
/// `id` is a stable key (app id / discovered path). Returns the PNG path.
pub fn cache_app_icon(icon_dir: &Path, id: &str, icon_src: &str) -> Option<PathBuf> {
    if let Ok(g) = mem_cache().lock() {
        if let Some(hit) = g.get(id) {
            return hit.clone();
        }
    }

    let out = icon_dir.join(format!("{id_hash}.png", id_hash = hash_id(id)));
    if out.is_file() {
        if let Ok(mut g) = mem_cache().lock() {
            g.insert(id.to_string(), Some(out.clone()));
        }
        return Some(out);
    }

    let (src, index) = split_icon_src(icon_src);
    let png = extract_icon_png(Path::new(&src), index)?;
    std::fs::create_dir_all(icon_dir).ok()?;
    std::fs::write(&out, &png).ok()?;
    if let Ok(mut g) = mem_cache().lock() {
        g.insert(id.to_string(), Some(out.clone()));
    }
    Some(out)
}

/// Drop the in-memory entry so a changed source is re-extracted.
#[allow(dead_code)]
pub fn invalidate_icon(id: &str) {
    if let Ok(mut g) = mem_cache().lock() {
        g.remove(id);
    }
}

fn hash_id(id: &str) -> String {
    // FNV-1a 64 — stable across runs, no extra dep.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn split_icon_src(src: &str) -> (String, Option<i32>) {
    if let Some((p, idx)) = src.rsplit_once(',') {
        if let Ok(i) = idx.trim().parse::<i32>() {
            return (p.trim().to_string(), Some(i));
        }
    }
    (src.trim().to_string(), None)
}

fn extract_icon_png(path: &Path, index: Option<i32>) -> Option<Vec<u8>> {
    // UWP logo / cached image: bytes already are the icon.
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".bmp")
    {
        if path.is_file() {
            return std::fs::read(path).ok();
        }
    }
    // shell: namespace targets do not exist as filesystem paths.
    let is_shell = lower.trim_start().starts_with("shell:");
    unsafe {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::UI::Shell::{ExtractIconExW, SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
        use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

        if !is_shell && !path.exists() {
            return None;
        }

        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let pcw = windows::core::PCWSTR(wide.as_ptr());

        // 1) Shell large icon
        let mut shfi = std::mem::zeroed::<SHFILEINFOW>();
        let ok = SHGetFileInfoW(
            pcw,
            Default::default(),
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if ok != 0 && !shfi.hIcon.is_invalid() {
            let png = hicon_to_png(shfi.hIcon);
            let _ = DestroyIcon(shfi.hIcon);
            if png.is_some() {
                return png;
            }
        }

        // 2) ExtractIconEx (supports `path,index` / negative resource id)
        let idx = index.unwrap_or(0);
        let mut large = [HICON::default(); 1];
        let n = ExtractIconExW(pcw, idx, Some(large.as_mut_ptr()), None, 1);
        if n > 0 && !large[0].is_invalid() {
            let png = hicon_to_png(large[0]);
            let _ = DestroyIcon(large[0]);
            return png;
        }
        None
    }
}

unsafe fn hicon_to_png(
    hicon: windows::Win32::UI::WindowsAndMessaging::HICON,
) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetIconInfo;

    let mut info = std::mem::zeroed();
    if GetIconInfo(hicon, &mut info).is_err() {
        return None;
    }
    let color_bmp = info.hbmColor;
    let mask_bmp = info.hbmMask;
    let use_mask = color_bmp.is_invalid();
    let src_bmp = if use_mask { mask_bmp } else { color_bmp };
    if src_bmp.is_invalid() {
        if !color_bmp.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(color_bmp.0));
        }
        if !mask_bmp.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(mask_bmp.0));
        }
        return None;
    }

    let mut bm = std::mem::zeroed::<windows::Win32::Graphics::Gdi::BITMAP>();
    if windows::Win32::Graphics::Gdi::GetObjectW(
        HGDIOBJ(src_bmp.0),
        std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAP>() as i32,
        Some(&mut bm as *mut _ as *mut _),
    ) == 0
    {
        let _ = DeleteObject(HGDIOBJ(color_bmp.0));
        let _ = DeleteObject(HGDIOBJ(mask_bmp.0));
        return None;
    }

    let w = bm.bmWidth as u32;
    let h = bm.bmHeight as u32;
    if w == 0 || h == 0 || w > 256 || h > 256 {
        let _ = DeleteObject(HGDIOBJ(color_bmp.0));
        let _ = DeleteObject(HGDIOBJ(mask_bmp.0));
        return None;
    }

    // Mask-only icons are 2× height (AND + XOR)
    let pixel_h = if use_mask { h / 2 } else { h };

    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(pixel_h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0 as u32,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut pixels = vec![0u8; (w * pixel_h * 4) as usize];
    let hdc = GetDC(None);
    let mem_dc = CreateCompatibleDC(hdc);
    let got = GetDIBits(
        mem_dc,
        windows::Win32::Graphics::Gdi::HBITMAP(src_bmp.0),
        0,
        pixel_h,
        Some(pixels.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    let _ = DeleteDC(mem_dc);
    let _ = ReleaseDC(None, hdc);
    let _ = DeleteObject(HGDIOBJ(color_bmp.0));
    let _ = DeleteObject(HGDIOBJ(mask_bmp.0));
    if got == 0 {
        return None;
    }

    // BGRA → RGBA, force opaque when alpha is 0 but color is present
    let mut rgba = Vec::with_capacity((w * pixel_h * 4) as usize);
    for px in pixels.chunks_exact(4) {
        let (b, g, r, mut a) = (px[0], px[1], px[2], px[3]);
        if a == 0 && (r | g | b) != 0 {
            a = 255;
        }
        rgba.extend_from_slice(&[r, g, b, a]);
    }

    let img = image::RgbaImage::from_raw(w, pixel_h, rgba)?;
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}
