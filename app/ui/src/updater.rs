//! Sparkle 2 업데이트 확인. 공개 배포 빌드만 `Contents/Frameworks/Sparkle.framework`를 포함하므로,
//! 프레임워크를 링크하지 않고 실행 중에 불러온다. 개발 빌드에는 프레임워크가 없어 메뉴도 만들지 않는다.
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::msg_send;
use objc2_foundation::{NSBundle, NSString};

/// 업데이터를 시작하고 `checkForUpdates:`를 받을 컨트롤러를 돌려준다. 앱이 끝날 때까지 유지한다.
pub(crate) unsafe fn start() -> Option<&'static AnyObject> {
    let main = NSBundle::mainBundle();
    let path = main.bundlePath().to_string() + "/Contents/Frameworks/Sparkle.framework";
    let framework = NSBundle::bundleWithPath(&NSString::from_str(&path))?;
    if !framework.load() {
        eprintln!("Sparkle.framework를 불러오지 못했습니다: {path}");
        return None;
    }
    let class = AnyClass::get(c"SPUStandardUpdaterController")?;
    let allocated: Allocated<AnyObject> = msg_send![class, alloc];
    let controller: Option<Retained<AnyObject>> = msg_send![
        allocated,
        initWithStartingUpdater: Bool::YES,
        updaterDelegate: std::ptr::null::<AnyObject>(),
        userDriverDelegate: std::ptr::null::<AnyObject>()
    ];
    controller.map(|controller| &*Retained::into_raw(controller))
}
