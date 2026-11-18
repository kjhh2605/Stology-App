//! Headless AppKit geometry regression; does not show windows or acquire focus.
#[path = "../src/font.rs"] mod font;
#[path = "../src/theme.rs"] mod theme;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
fn rect(x:f64,y:f64,w:f64,h:f64)->NSRect {NSRect::new(NSPoint::new(x,y),NSSize::new(w,h))}
fn main(){unsafe{
 let mtm=MainThreadMarker::new().unwrap();
 let _app=NSApplication::sharedApplication(mtm);
 // Demonstrate the original ownership mismatch without displaying a window.
 let (legacy_shell,legacy)=theme::panel(rect(0.,0.,340.,740.));
 legacy.setFrame(rect(0.,0.,340.,686.));
 legacy_shell.layoutSubtreeIfNeeded();
 println!("Legacy direct content height after native layout: {} (requested 686)",legacy.frame().size.height);
 let legacy_header=NSTextField::labelWithString(&NSString::from_str("자료 보관함"),mtm);
 legacy_header.setFrame(rect(44.,640.,244.,20.));legacy.addSubview(&legacy_header);
 for i in 0..16 {
  let mut f=legacy_header.frame();f.origin.y+=686.-legacy.bounds().size.height;
  legacy_header.setFrame(f);
  let width=if i%2==0 {220.}else{340.};
  legacy_shell.setFrame(rect(0.,0.,width,740.));
  legacy.setFrame(rect(0.,0.,width,686.));
  legacy_shell.layoutSubtreeIfNeeded();
 }
 println!("Legacy header y after 16 resize/layout cycles: {} (started at 640)",legacy_header.frame().origin.y);


 let (shell,host,body)=theme::panel_with_body(rect(0.,0.,340.,740.),54.);
 let header=NSTextField::labelWithString(&NSString::from_str("자료 보관함"),mtm);
 header.setFrame(rect(44.,640.,244.,20.));
 let search=NSSearchField::initWithFrame(NSSearchField::alloc(mtm),rect(16.,596.,308.,32.));
 search.setStringValue(&NSString::from_str("검색어 유지"));
 let draft=NSTextField::labelWithString(&NSString::from_str("작성 중"),mtm);
 draft.setFrame(rect(42.,180.,244.,20.));
 body.addSubview(&header);body.addSubview(&search);body.addSubview(&draft);
 for _ in 0..10 {
  for (width,height) in [(340.,740.),(516.,740.),(220.,740.),(220.,620.),(480.,840.),(220.,620.)] {
   let h=height-54.;
   // Match the app's incremental height layout before changing panel frames.
   let delta=h-body.bounds().size.height;
   for child in body.subviews().iter(){
    let mut f=child.frame();f.origin.y+=delta;child.setFrame(f);
    child.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
   }
   shell.setFrame(rect(0.,0.,width,height));
   body.setFrame(rect(0.,0.,width,h));
   shell.layoutSubtreeIfNeeded();
   assert_eq!(host.frame().size,shell.bounds().size);
   assert_eq!(body.frame().size,NSSize::new(width,h));
   assert_eq!(header.frame().origin.y,h-46.);
   assert_eq!(search.frame().origin.y,h-90.);
   for child in body.subviews().iter(){
    let f=child.frame();assert!(f.origin.y>=0. && f.origin.y+f.size.height<=h);
    assert!(f.size.width>0. && f.origin.x+f.size.width<=width);
   }
   assert_eq!(search.stringValue().to_string(),"검색어 유지");
  }
 }
 println!("PASS: 60 resize/layout cycles; full-size native host; stable body height; headers/search remain visible; search text preserved.");
}}
