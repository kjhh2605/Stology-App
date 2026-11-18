import AppKit
import PDFKit
import ImageIO
let out = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory:true)
let image = NSImage(size:NSSize(width:640,height:400))
image.lockFocus()
NSColor.white.setFill(); NSRect(x:0,y:0,width:640,height:400).fill()
NSColor(calibratedRed:0.31,green:0.27,blue:0.90,alpha:1).setFill(); NSBezierPath(ovalIn:NSRect(x:65,y:135,width:130,height:130)).fill()
NSColor(calibratedRed:0.96,green:0.62,blue:0.04,alpha:1).setFill(); NSRect(x:430,y:140,width:120,height:120).fill()
NSColor.black.setStroke();let arrow=NSBezierPath();arrow.lineWidth=8;arrow.move(to:NSPoint(x:225,y:200));arrow.line(to:NSPoint(x:390,y:200));arrow.move(to:NSPoint(x:375,y:216));arrow.line(to:NSPoint(x:391,y:200));arrow.line(to:NSPoint(x:375,y:184));arrow.stroke()
image.unlockFocus()
let bitmap=NSBitmapImageRep(data:image.tiffRepresentation!)!
try bitmap.representation(using:.png,properties:[:])!.write(to:out.appendingPathComponent("diagram.png"))
try bitmap.representation(using:.jpeg,properties:[.compressionFactor:0.9])!.write(to:out.appendingPathComponent("diagram.jpg"))
if let dest=CGImageDestinationCreateWithURL(out.appendingPathComponent("diagram.heic") as CFURL,"public.heic" as CFString,1,nil){CGImageDestinationAddImage(dest,bitmap.cgImage!,nil);if !CGImageDestinationFinalize(dest){fatalError("HEIC encode failed")}}
let pdf=PDFDocument();pdf.insert(PDFPage(image:image)!,at:0);pdf.write(to:out.appendingPathComponent("diagram-scan.pdf"))
var box=CGRect(x:0,y:0,width:595,height:842)
let ctx=CGContext(out.appendingPathComponent("biology.pdf") as CFURL,mediaBox:&box,nil)!
ctx.beginPDFPage(nil)
let old=NSGraphicsContext.current;NSGraphicsContext.current=NSGraphicsContext(cgContext:ctx,flipped:false)
("세포와 에너지\n\n엽록체는 광합성이 일어나는 세포 소기관이다.\n빛이 있을 때 광합성이 일어난다.\n미토콘드리아는 세포 호흡을 통해 ATP를 생성한다." as NSString).draw(in:NSRect(x:48,y:570,width:500,height:220),withAttributes:[.font:NSFont.systemFont(ofSize:20),.foregroundColor:NSColor.black])
NSGraphicsContext.current=old;ctx.endPDFPage();ctx.closePDF()
print("PDF, PNG, JPEG, HEIC fixtures generated")
