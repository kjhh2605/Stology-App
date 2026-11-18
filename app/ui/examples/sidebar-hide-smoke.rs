//! Regression for the actual hide renderer: paint above the system material.
//! Default mode renders offscreen and checks pixels; --show displays the same
//! view at six drag offsets for native visual inspection, without user data.
#[path = "../src/sidebar_hide.rs"] mod sidebar_hide;
use objc2::{msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::*;
use objc2_foundation::*;
fn rect(x:f64,y:f64,w:f64,h:f64)->NSRect {NSRect::new(NSPoint::new(x,y),NSSize::new(w,h))}
fn main(){unsafe{
 let left_side=std::env::args().any(|a|a=="--left");
 let panel_width=if left_side {196.} else {220.};
 let mtm=MainThreadMarker::new().unwrap();let app=NSApplication::sharedApplication(mtm);
 let window=NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm),
  rect(100.,100.,780.,740.),NSWindowStyleMask::Titled|NSWindowStyleMask::Closable,NSBackingStoreType::Buffered,false);
 window.setReleasedWhenClosed(false);window.setTitle(&NSString::from_str("라벤더 숨김 · 실제 렌더러 검증"));
 window.setAppearance(NSAppearance::appearanceNamed(NSAppearanceNameAqua).as_deref());
 let root=NSView::initWithFrame(NSView::alloc(mtm),rect(0.,0.,780.,740.));
 root.setWantsLayer(true);let layer:Retained<AnyObject>=msg_send![&root,layer];
 let color=NSColor::colorWithSRGBRed_green_blue_alpha(0.94,0.95,0.99,1.);
 let _:()=msg_send![&layer,setBackgroundColor:&*color.CGColor()];window.setContentView(Some(&root));
 for (i,offset) in [0.,panel_width-80.,panel_width-24.,panel_width-12.,panel_width-4.,panel_width].into_iter().enumerate(){
  let x=30.+250.*(i%3) as f64;
  let row=if i<3 {340.} else {0.};
  let label=NSTextField::labelWithString(&NSString::from_str(&format!("{}pt 고정 · {} {offset}pt 이동",panel_width,if left_side {"왼쪽"} else {"오른쪽"})),mtm);
  label.setFrame(rect(x,350.+row,230.,24.));root.addSubview(&label);
  let clip=NSView::initWithFrame(NSView::alloc(mtm),rect(x,30.+row,panel_width,300.));clip.setWantsLayer(true);
  let l:Retained<AnyObject>=msg_send![&clip,layer];let _:()=msg_send![&l,setMasksToBounds:true];root.addSubview(&clip);
  let underlying=NSView::initWithFrame(NSView::alloc(mtm),rect(if left_side {-offset} else {offset},0.,panel_width,300.));
  for (j,text) in ["자료 보관함","세포와 에너지","세포 소기관 정리"].iter().enumerate(){
   let t=NSTextField::labelWithString(&NSString::from_str(text),mtm);t.setFrame(rect(16.,260.-j as f64*36.,188.,24.));underlying.addSubview(&t);
  }
  underlying.setAlphaValue(0.45);clip.addSubview(&underlying);
  let veil=sidebar_hide::new(left_side);veil.setFrame(rect(if left_side {-offset} else {offset},0.,panel_width,300.));veil.setHidden(false);clip.addSubview(&veil);
  let cue=sidebar_hide::new_arrow(left_side);
  cue.setFrame(if left_side {sidebar_hide::left_arrow_frame(veil.frame(),0.)}
    else {sidebar_hide::arrow_frame(veil.frame(),panel_width)});
  cue.setHidden(false);clip.addSubview(&cue);
  assert_eq!(cue.frame().origin,NSPoint::new(if left_side {0.} else {offset},130.));
  assert_eq!(cue.frame().size.width,panel_width-offset);
  assert!(cue.frame().origin.x+cue.frame().size.width<=clip.bounds().size.width);
  root.layoutSubtreeIfNeeded();
  // Test the foreground ink itself: the old implementation painted nothing
  // because it put drawRect on NSVisualEffectView rather than a paint subview.
  let ink=veil.subviews().lastObject().unwrap();
  let bitmap=ink.bitmapImageRepForCachingDisplayInRect(ink.bounds()).expect("ink bitmap");
  ink.cacheDisplayInRect_toBitmapImageRep(ink.bounds(),&bitmap);
  let sx=bitmap.pixelsWide() as f64/panel_width;let sy=bitmap.pixelsHigh() as f64/300.;
  let pixel=|x:f64,y:f64|bitmap.colorAtX_y((x*sx)as isize,(y*sy)as isize).unwrap().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()).unwrap();
  let left=pixel(30.,60.);let right=pixel(panel_width-30.,60.);
  let composed=clip.bitmapImageRepForCachingDisplayInRect(clip.bounds()).expect("composed preview");
  clip.cacheDisplayInRect_toBitmapImageRep(clip.bounds(),&composed);
  let dark:Vec<f64>=(0..composed.pixelsWide()).filter_map(|px| {
    let c=composed.colorAtX_y(px,(150.*sy)as isize).unwrap()
      .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()).unwrap();
    (c.alphaComponent()>0.9 && c.redComponent()<0.55).then_some(px as f64/sx)
  }).collect();
  let (strong,weak)=if left_side {(&left,&right)} else {(&right,&left)};
  assert!(strong.blueComponent()>strong.greenComponent()+0.12,"lavender hue missing");
  assert!(strong.alphaComponent()>weak.alphaComponent()+0.2,"opacity gradient direction missing");
  if offset==panel_width {assert!(dark.is_empty(),"zero-width area must hide arrow");}
  else {
    let remaining=panel_width-offset;
    let expected_left=if left_side {(remaining-((remaining-24.)/2.).max(0.)-23.).max(0.)}
        else {offset+((remaining-24.)/2.).max(0.)+1.};
    assert!((dark[0]-expected_left).abs()<2.,"arrow must follow remaining-area center: {offset}, {dark:?}");
    let visible_width=dark.len() as f64/sx;
    let expected_width=(remaining-1.).min(22.);
    assert!((visible_width-expected_width).abs()<2.,"outer edge must clip without shrinking: {offset}, {visible_width}");
  }
 }
 println!("PASS: arrow follows remaining-area center; full-size glyph clips at the outer edge for 24/12/4/0pt remaining widths; lavender preserved.");
 if std::env::args().any(|a|a=="--show"){
  app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
  let bar=NSMenu::new(mtm);let item=NSMenuItem::new(mtm);let menu=NSMenu::new(mtm);
  let quit=NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm),&NSString::from_str("Quit"),Some(sel!(terminate:)),&NSString::from_str("q"));
  menu.addItem(&quit);item.setSubmenu(Some(&menu));bar.addItem(&item);app.setMainMenu(Some(&bar));
  window.makeKeyAndOrderFront(None);app.run();
 }
}}
