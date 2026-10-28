//! Files dragged onto the pill. Tauri registers its drop handler on the webview's windows when the pill is created,
//! but WebView2 often makes the pill's inner windows (the ones under the cursor) a moment later, so Windows found no
//! drop target there and refused the file. `watch` adds ours to any of the pill's windows that lack one.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use windows::core::{implement, w, Ref, BOOL};
use windows::Win32::Foundation::{HWND, LPARAM, POINTL};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetPropW};

use crate::dictation::Control;
use crate::App;

/// The files being dragged, if any.
fn files(data: Ref<'_, IDataObject>) -> Vec<PathBuf> {
    let Some(data) = data.as_ref() else { return Vec::new() };
    let format = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    unsafe {
        let Ok(mut medium) = data.GetData(&format) else { return Vec::new() }; // not files (text, a link)
        let hdrop = HDROP(medium.u.hGlobal.0 as _);
        let paths = (0..DragQueryFileW(hdrop, u32::MAX, None))
            .map(|i| {
                let mut buf = vec![0u16; DragQueryFileW(hdrop, i, None) as usize + 1];
                let n = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
                PathBuf::from(OsString::from_wide(&buf[..n]))
            })
            .collect();
        ReleaseStgMedium(&mut medium);
        paths
    }
}

#[implement(IDropTarget)]
struct Target(AppHandle);

impl Target {
    fn tell(&self, c: Control) {
        self.0.state::<App>().tell(c);
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<'_, IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let ok = !files(data).is_empty();
        if ok {
            self.tell(Control::DropHover(true));
        }
        unsafe { *effect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        Ok(())
    }

    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        // DragEnter already refused anything that isn't files; Windows keeps that answer.
        unsafe { *effect = DROPEFFECT_COPY };
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.tell(Control::DropHover(false));
        Ok(())
    }

    fn Drop(&self, data: Ref<'_, IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        match files(data).into_iter().next() {
            Some(path) => self.tell(Control::FileDropped(path)),
            None => self.tell(Control::DropHover(false)),
        }
        unsafe { *effect = DROPEFFECT_COPY };
        Ok(())
    }
}

/// Every few seconds, gives the pill's windows that have no drop target ours (WebView2 can recreate them).
/// Registration runs on the main thread, which Tauri set up for OLE.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || loop {
        let a = app.clone();
        let _ = app.run_on_main_thread(move || register(&a));
        std::thread::sleep(Duration::from_secs(3));
    });
}

fn register(app: &AppHandle) {
    let Some(pill) = app.get_webview_window("pill").and_then(|w| w.hwnd().ok()) else { return };
    let target: IDropTarget = Target(app.clone()).into();
    let mut missing: Vec<HWND> = Vec::new();
    unsafe extern "system" fn each(h: HWND, list: LPARAM) -> BOOL {
        if GetPropW(h, w!("OleDropTargetInterface")).is_invalid() {
            (*(list.0 as *mut Vec<HWND>)).push(h);
        }
        true.into()
    }
    unsafe {
        let _ = EnumChildWindows(Some(HWND(pill.0 as _)), Some(each), LPARAM(&mut missing as *mut _ as isize));
        for h in missing {
            if RegisterDragDrop(h, &target).is_ok() {
                crate::log("pill: file drop target added");
            }
        }
    }
}
