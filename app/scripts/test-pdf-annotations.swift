import AppKit
import PDFKit
import Foundation

// Reproducible PDFKit gate. No app database, fixtures, or user documents are used.
// swiftc -O scripts/test-pdf-annotations.swift -o /tmp/stology-pdf-gate
// /tmp/stology-pdf-gate /tmp/stology-pdf-fixtures [--benchmark]
struct Failure: Error, CustomStringConvertible { let description: String }
func require(_ value: @autoclosure () -> Bool, _ message: String) throws {
    if !value() { throw Failure(description: message) }
}
let fm = FileManager.default
let root = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : NSTemporaryDirectory()+"stology-pdf-gate-"+UUID().uuidString, isDirectory: true)
let stableID = PDFAnnotationKey(rawValue: "/StologyUUID")
let schema = PDFAnnotationKey(rawValue: "/StologySchema")
let payload = PDFAnnotationKey(rawValue: "/StologyPayload")
let foreign = PDFAnnotationKey(rawValue: "/ForeignPayload")
let marker = "세포 에너지 한글 메모 받침 수정 ABC 123"
let baselineText = "Stology PDF preservation fixture. Body content remains independently selectable after annotation edits."
func save(_ doc: PDFDocument, _ name: String) throws -> PDFDocument {
    let url = root.appendingPathComponent(name)
    try require(doc.write(to: url), "write failed: \(name)")
    guard let reopened = PDFDocument(url: url) else { throw Failure(description: "reopen failed: \(name)") }
    return reopened
}
func makeBase(_ url: URL, pages: Int, noisy: Bool = false) throws {
    var media = CGRect(x: 0, y: 0, width: 612, height: 792)
    guard let ctx = CGContext(url as CFURL, mediaBox: &media, nil) else { throw Failure(description: "CGContext") }
    for i in 0..<pages {
        ctx.beginPDFPage(nil)
        let previous = NSGraphicsContext.current
        NSGraphicsContext.current = NSGraphicsContext(cgContext: ctx, flipped: false)
        (baselineText as NSString).draw(in: CGRect(x: 50, y: 650, width: 500, height: 80), withAttributes: [.font: NSFont.systemFont(ofSize: 16), .foregroundColor: NSColor.black])
        if noisy {
            // Deterministic image data forces meaningful serialization bytes, without huge file fixtures in git.
            let side = 768
            var state = UInt64(i + 1)
            var pixels = [UInt8](repeating: 0, count: side*side*4)
            for j in stride(from: 0, to: pixels.count, by: 4) {
                state = state &* 6364136223846793005 &+ 1
                pixels[j] = UInt8(truncatingIfNeeded: state >> 32)
                pixels[j+1] = UInt8(truncatingIfNeeded: state >> 40)
                pixels[j+2] = UInt8(truncatingIfNeeded: state >> 48)
                pixels[j+3] = 255
            }
            let provider = CGDataProvider(data: Data(pixels) as CFData)!
            let image = CGImage(width: side, height: side, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: side*4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue), provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
            ctx.draw(image, in: CGRect(x: 40, y: 70, width: 530, height: 530))
        }
        NSGraphicsContext.current = previous
        ctx.endPDFPage()
    }
    ctx.closePDF()
}
func id(_ annotation: PDFAnnotation) -> String { annotation.value(forAnnotationKey: stableID) as? String ?? annotation.value(forAnnotationKey: .name) as? String ?? "" }
func appAnnotation(_ type: PDFAnnotationSubtype, _ bounds: CGRect, _ identifier: String) throws -> PDFAnnotation {
    let a = PDFAnnotation(bounds: bounds, forType: type, withProperties: nil)
    try require(a.setValue(identifier, forAnnotationKey: .name), "set /NM")
    try require(a.setValue(identifier, forAnnotationKey: stableID), "set UUID fallback")
    try require(a.setValue(1, forAnnotationKey: schema), "set schema")
    try require(a.setValue("{\"tool\":\"\(type.rawValue)\",\"text\":\"\(marker)\"}", forAnnotationKey: payload), "set payload")
    a.shouldPrint = true
    return a
}
func runGate() throws -> [String: Any] {
    let baseURL = root.appendingPathComponent("base.pdf")
    try makeBase(baseURL, pages: 4)
    let base = PDFDocument(url: baseURL)!
    for i in 0..<4 {
        let p = base.page(at: i)!
        p.rotation = i*90
        p.setBounds(CGRect(x: 20, y: 30, width: 560, height: 740), for: .cropBox)
    }
    let page = base.page(at: 0)!
    let link = PDFAnnotation(bounds: CGRect(x: 50, y: 620, width: 200, height: 25), forType: .link, withProperties: nil)
    link.action = PDFActionURL(url: URL(string: "https://example.com/pdf-preservation")!)
    link.setValue("foreign-link", forAnnotationKey: stableID)
    page.addAnnotation(link)
    let form = PDFAnnotation(bounds: CGRect(x: 50, y: 580, width: 200, height: 25), forType: .widget, withProperties: nil)
    form.widgetFieldType = .text
    form.fieldName = "existing-form"
    form.widgetStringValue = "기존 입력값"
    form.setValue("foreign-form", forAnnotationKey: stableID)
    page.addAnnotation(form)
    let stamp = PDFAnnotation(bounds: CGRect(x: 400, y: 580, width: 100, height: 30), forType: .stamp, withProperties: nil)
    stamp.contents = "Preserve foreign stamp"
    stamp.setValue("foreign-stamp", forAnnotationKey: stableID)
    stamp.setValue("do-not-discard", forAnnotationKey: foreign)
    page.addAnnotation(stamp)
    let unknown = PDFAnnotation(bounds: CGRect(x: 500, y: 500, width: 25, height: 25), forType: PDFAnnotationSubtype(rawValue: "/ForeignReview"), withProperties: nil)
    unknown.setValue("foreign-unknown", forAnnotationKey: stableID)
    unknown.setValue("opaque-vendor-data", forAnnotationKey: foreign)
    page.addAnnotation(unknown)
    let source = try save(base, "source.pdf")
    let originalText = source.string
    let ids = (0..<4).map { _ in UUID().uuidString }
    final class WorkerResult: @unchecked Sendable { var error: Error? }
    let workerResult = WorkerResult()
    let finished = DispatchSemaphore(value: 0)
    DispatchQueue(label: "stology.pdf.prototype.worker").async {
        defer { finished.signal() }
        do {
        try require(!Thread.isMainThread, "worker executed on main thread")
        // Own PDFDocument on a private worker, loaded independently from the UI/source object.
        let doc = PDFDocument(url: root.appendingPathComponent("source.pdf"))!
        for i in 0..<4 {
            let p = doc.page(at: i)!
            let note = try appAnnotation(.freeText, CGRect(x: 60, y: 400, width: 390, height: 90), ids[i])
            note.contents = marker
            note.font = NSFont.systemFont(ofSize: 22)
            note.fontColor = .black
            note.color = NSColor.yellow.withAlphaComponent(0.2)
            p.addAnnotation(note)
            let highlight = try appAnnotation(.highlight, CGRect(x: 50, y: 650, width: 400, height: 20), "highlight-\(i)")
            highlight.color = NSColor.yellow.withAlphaComponent(0.4)
            highlight.quadrilateralPoints = [NSPoint(x: 0,y: 20),NSPoint(x:400,y:20),NSPoint(x:0,y:0),NSPoint(x:400,y:0)].map { NSValue(point:$0) }
            p.addAnnotation(highlight)
            let box = try appAnnotation(.square, CGRect(x: 60, y: 300, width: 200, height: 60), "box-\(i)")
            box.color = .red
            box.border = PDFBorder(); box.border!.lineWidth = 3
            p.addAnnotation(box)
            let ink = try appAnnotation(.ink, CGRect(x: 60, y: 180, width: 200, height: 70), "ink-\(i)")
            let path = NSBezierPath(); path.move(to: NSPoint(x:0,y:0)); path.line(to:NSPoint(x:70,y:50));path.line(to:NSPoint(x:180,y:20))
            ink.add(path); ink.color = .blue
            p.addAnnotation(ink)
        }
        _ = try save(doc, "annotated.pdf")
        } catch { workerResult.error = error }
    }
    finished.wait()
    if let error = workerResult.error { throw error }
    let edited = PDFDocument(url: root.appendingPathComponent("annotated.pdf"))!
    try require(edited.pageCount == source.pageCount, "page count changed")
    try require(edited.string == originalText, "body text changed")
    for i in 0..<4 {
        let p = edited.page(at:i)!
        try require(p.rotation == i*90, "rotation changed")
        try require(p.bounds(for:.cropBox) == CGRect(x:20,y:30,width:560,height:740), "crop changed")
        let note = p.annotations.first { id($0) == ids[i] }
        try require(note != nil, "UUID lost")
        try require(note?.contents == marker, "Korean note lost")
        try require((note?.value(forAnnotationKey:schema) as? NSNumber)?.intValue == 1, "schema lost")
        try require((note?.value(forAnnotationKey:payload) as? String)?.contains(marker) == true, "payload lost")
        let highlight = p.annotations.first { id($0) == "highlight-\(i)" }!
        try require(highlight.quadrilateralPoints?.count == 4, "quads lost")
        try require(highlight.quadrilateralPoints?.first?.pointValue == NSPoint(x:0,y:20), "quad coordinates changed")
        let ink = p.annotations.first { id($0) == "ink-\(i)" }!
        try require(ink.paths?.count == 1 && ink.paths?.first?.elementCount == 3, "ink geometry lost")
    }
    let changed = edited.page(at:0)!.annotations.first { id($0) == ids[0] }!
    changed.contents = marker + " 수정됨"
    changed.bounds.origin.x += 15
    let deleted = edited.page(at:1)!.annotations.first { id($0) == "box-1" }!
    edited.page(at:1)!.removeAnnotation(deleted)
    let final = try save(edited, "edited.pdf")
    try require(final.page(at:0)!.annotations.first { id($0) == ids[0] }?.contents == marker + " 수정됨", "edit not persisted")
    try require(!final.page(at:1)!.annotations.contains { id($0) == "box-1" }, "delete not persisted")
    let finalPage = final.page(at:0)!
    try require((finalPage.annotations.first { id($0) == "foreign-link" }?.action as? PDFActionURL)?.url?.absoluteString == "https://example.com/pdf-preservation", "link lost")
    try require(finalPage.annotations.first { id($0) == "foreign-form" }?.widgetStringValue == "기존 입력값", "form value lost")
    try require(finalPage.annotations.first { id($0) == "foreign-form" }?.fieldName == "existing-form", "form field lost")
    try require(finalPage.annotations.first { id($0) == "foreign-stamp" }?.value(forAnnotationKey:foreign) as? String == "do-not-discard", "foreign metadata lost")
    try require(finalPage.annotations.first { id($0) == "foreign-unknown" }?.type == "ForeignReview", "unknown annotation subtype lost")
    try require(finalPage.annotations.first { id($0) == "foreign-unknown" }?.value(forAnnotationKey:foreign) as? String == "opaque-vendor-data", "unknown annotation data lost")
    try require(final.string == originalText, "body text changed after edit/delete")
    // An independent document reads only the PDF; no sidecar/journal exists.
    return ["gate":"PASS", "pages":4, "rotations":[0,90,180,270], "nonzeroCropBox":true, "UUID":true, "UUIDStorage":"/StologyUUID (standard /NM dropped by PDFKit)", "customMetadata":true, "KoreanFreeText":true, "ink":true, "quadCoordinates":true, "link":true, "form":true, "foreignStamp":true, "unknownSubtype":true, "bodyText":true, "workerOwnedDocument":true, "standaloneReeditDelete":true, "fixtures":root.path]
}
func benchmark(pages: Int, noisy: Bool, iterations: Int) throws -> [String: Any] {
    let input = root.appendingPathComponent("benchmark-\(pages).pdf")
    try makeBase(input, pages: pages, noisy: noisy)
    let output = root.appendingPathComponent("benchmark-\(pages)-saved.pdf")
    let doc = PDFDocument(url: input)!
    let annotation = try appAnnotation(.square, CGRect(x:40,y:40,width:100,height:100), UUID().uuidString)
    doc.page(at:0)!.addAnnotation(annotation)
    var times: [Double] = [], sizes: [Int] = []
    for n in 0..<iterations {
        annotation.bounds.origin.x = CGFloat(40+n)
        let start = ContinuousClock.now
        try require(doc.write(to:output), "benchmark write")
        let duration = start.duration(to: .now).components
        times.append(Double(duration.seconds)*1000 + Double(duration.attoseconds)/1e15)
        sizes.append((try fm.attributesOfItem(atPath: output.path)[.size] as! NSNumber).intValue)
        try require(PDFDocument(url:output)?.pageCount == pages, "benchmark reopen")
    }
    return ["pages":pages,"annotations":1,"iterations":iterations,"inputBytes":(try fm.attributesOfItem(atPath:input.path)[.size] as! NSNumber).intValue,"writeMilliseconds":times,"outputBytes":sizes,"sumOutputBytes":sizes.reduce(0,+)]
}
func opacityRasterStats(_ page: PDFPage) -> [String: Int] {
    let context = CGContext(data:nil, width:612, height:792, bitsPerComponent:8, bytesPerRow:612*4,
        space:CGColorSpaceCreateDeviceRGB(), bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue)!
    context.setFillColor(CGColor(gray:1,alpha:1))
    context.fill(CGRect(x:0,y:0,width:612,height:792))
    page.draw(with:.mediaBox,to:context)
    let bytes = UnsafeBufferPointer(start:context.data!.assumingMemoryBound(to:UInt8.self),count:612*792*4)
    var dark = 0, blues: [Int] = []
    for index in stride(from:0,to:bytes.count,by:4) {
        if bytes[index] < 96 && bytes[index+1] < 96 && bytes[index+2] < 96 { dark += 1 }
        if bytes[index] > 180 && bytes[index+1] > 180 && bytes[index+2] < 245 { blues.append(Int(bytes[index+2])) }
    }
    blues.sort()
    return ["darkPixels":dark,"yellowPixels":blues.count,"yellowMedianBlue":blues.isEmpty ? -1 : blues[blues.count/2]]
}
func verifyOpacity() throws -> [String: Any] {
    let original = PDFDocument(url:root.appendingPathComponent("base.pdf"))!
    let baseline = opacityRasterStats(original.page(at:0)!)
    var result: [String: Any] = ["baseline":baseline]
    for kind in ["highlight","area","box"] {
        let document = PDFDocument(url:root.appendingPathComponent("base.pdf"))!
        let page = document.page(at:0)!
        let annotation = PDFAnnotation(bounds:CGRect(x:50,y:650,width:500,height:80),forType:kind == "highlight" ? .highlight : .square,withProperties:nil)
        if kind == "highlight" {
            annotation.color = NSColor.yellow.withAlphaComponent(0.25)
            annotation.quadrilateralPoints = [NSPoint(x:0,y:80),NSPoint(x:500,y:80),NSPoint(x:0,y:0),NSPoint(x:500,y:0)].map { NSValue(point:$0) }
        } else if kind == "area" {
            annotation.color = .clear
            annotation.interiorColor = NSColor.yellow.withAlphaComponent(0.25)
        } else {
            annotation.color = .red
            annotation.border = PDFBorder()
            annotation.border!.lineWidth = 2
        }
        page.addAnnotation(annotation)
        let before = opacityRasterStats(page)
        let reopened = try save(document,"opacity-\(kind).pdf")
        let after = opacityRasterStats(reopened.page(at:0)!)
        try require(after["darkPixels"]! > baseline["darkPixels"]! / 2,"\(kind) obscured body text")
        try require(abs(before["darkPixels"]! - after["darkPixels"]!) < 10,"\(kind) body visibility changed after saving")
        if kind == "area" {
            try require(after["yellowMedianBlue"] == 191,"area fill lost alpha 0.25 appearance")
        }
        result[kind] = ["before":before,"after":after]
    }
    return result
}
do {
    try fm.createDirectory(at:root, withIntermediateDirectories:true)
    var result = try runGate()
    result["opacityRaster"] = try verifyOpacity()
    if CommandLine.arguments.contains("--benchmark") {
        result["benchmarks"] = try [benchmark(pages:4,noisy:false,iterations:5), benchmark(pages:32,noisy:true,iterations:5)]
    }
    let data = try JSONSerialization.data(withJSONObject:result,options:[.prettyPrinted,.sortedKeys])
    print(String(data:data,encoding:.utf8)!)
} catch { FileHandle.standardError.write(Data("FAIL: \(error)\n".utf8)); exit(1) }
