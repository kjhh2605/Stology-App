import AppKit
import Foundation
import ImageIO
import PDFKit
import Vision

// An isolated system-framework worker. It never writes beside the source file.
// Usage: stology-extract <source path> <dedicated cache directory>
// stdout is one JSON object; all failures go to stderr with a nonzero exit code.
struct ExtractionFailure: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

let maximumInputBytes: UInt64 = 100 * 1024 * 1024
let maximumOutputBytes = 128 * 1024 * 1024
let maximumPixels = 100_000_000
let maximumDimension = 1600

func recognize(_ image: CGImage) throws -> String {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    let supported = try request.supportedRecognitionLanguages()
    let languages = ["ko-KR", "en-US"].filter { supported.contains($0) }
    if !languages.isEmpty { request.recognitionLanguages = languages }
    do {
        try VNImageRequestHandler(cgImage: image, options: [:]).perform([request])
    } catch {
        throw ExtractionFailure("시스템 OCR 실패: \(error.localizedDescription)")
    }
    // Vision returns observations in reading order. Coordinates refer to the
    // raster supplied to the model, so text and the visual input stay aligned.
    return (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
        .joined(separator: "\n")
}

func encodePNG(_ image: CGImage, at url: URL, totalBytes: inout Int) throws {
    let data = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(data, "public.png" as CFString, 1, nil) else {
        throw ExtractionFailure("시스템 PNG 인코더를 시작할 수 없습니다.")
    }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else {
        throw ExtractionFailure("미리보기 PNG를 만들 수 없습니다.")
    }
    guard totalBytes <= maximumOutputBytes - data.length else {
        throw ExtractionFailure("추출 이미지 캐시가 128 MiB 제한을 초과합니다. 자료를 나누어 추가하세요.")
    }
    // The caller owns this private, content-addressed cache. Atomic replacement
    // permits a retry after a partial extraction without exposing partial PNGs.
    do { try (data as Data).write(to: url, options: [.atomic]) }
    catch { throw ExtractionFailure("추출 이미지 저장 실패: \(error.localizedDescription)") }
    totalBytes += data.length
}

func raster(_ page: PDFPage) throws -> CGImage {
    guard let pageRef = page.pageRef else { throw ExtractionFailure("PDF 페이지를 읽을 수 없습니다.") }
    let bounds = pageRef.getBoxRect(.mediaBox)
    guard bounds.width.isFinite, bounds.height.isFinite, bounds.width > 0, bounds.height > 0,
          bounds.width < 1_000_000, bounds.height < 1_000_000 else {
        throw ExtractionFailure("PDF 페이지의 크기가 올바르지 않습니다.")
    }
    let rotated = abs(pageRef.rotationAngle) % 180 != 0
    let originalWidth = rotated ? bounds.height : bounds.width
    let originalHeight = rotated ? bounds.width : bounds.height
    let scale = min(CGFloat(maximumDimension) / max(originalWidth, originalHeight), 2)
    let width = max(1, Int(ceil(originalWidth * scale)))
    let height = max(1, Int(ceil(originalHeight * scale)))
    guard let context = CGContext(data: nil, width: width, height: height,
                                  bitsPerComponent: 8, bytesPerRow: width * 4,
                                  space: CGColorSpaceCreateDeviceRGB(),
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
        throw ExtractionFailure("PDF 이미지 메모리를 할당할 수 없습니다.")
    }
    let rect = CGRect(x: 0, y: 0, width: width, height: height)
    context.setFillColor(CGColor(gray: 1, alpha: 1))
    context.fill(rect)
    // PDFKit draws the editable annotations as well as page contents. Its draw
    // method already applies box origin and page rotation; applying the CGPDF
    // drawing transform here would rotate/translate the page a second time.
    context.scaleBy(x: CGFloat(width) / originalWidth, y: CGFloat(height) / originalHeight)
    page.displaysAnnotations = true
    page.draw(with: .mediaBox, to: context)
    guard let image = context.makeImage() else { throw ExtractionFailure("PDF 페이지 이미지를 만들 수 없습니다.") }
    return image
}

func decodeImage(_ url: URL) throws -> CGImage {
    guard let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
          CGImageSourceGetCount(source) > 0,
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int else {
        throw ExtractionFailure("손상되었거나 시스템이 디코딩할 수 없는 이미지입니다.")
    }
    guard width > 0, height > 0, width <= maximumPixels / height else {
        throw ExtractionFailure("이미지가 1억 픽셀 제한을 초과하거나 크기가 올바르지 않습니다.")
    }
    let options: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceThumbnailMaxPixelSize: maximumDimension,
        kCGImageSourceShouldCacheImmediately: true
    ]
    guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
        throw ExtractionFailure("이미지 픽셀을 디코딩하지 못했습니다. 파일이 손상되었을 수 있습니다.")
    }
    return image
}

func extract(source: URL, output: URL) throws -> [String: Any] {
    let manager = FileManager.default
    let info = try source.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
    guard info.isRegularFile == true else { throw ExtractionFailure("입력은 일반 파일이어야 합니다.") }
    guard let size = info.fileSize, size > 0, UInt64(size) <= maximumInputBytes else {
        throw ExtractionFailure("빈 파일이거나 100 MiB 입력 제한을 초과했습니다.")
    }
    try manager.createDirectory(at: output, withIntermediateDirectories: true)
    let outputInfo = try output.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
    guard outputInfo.isDirectory == true, outputInfo.isSymbolicLink != true else {
        throw ExtractionFailure("캐시는 심볼릭 링크가 아닌 전용 폴더여야 합니다.")
    }
    var totalBytes = 0
    var units: [[String: Any]] = []
    let ext = source.pathExtension.lowercased()
    if ext == "pdf" {
        guard let document = PDFDocument(url: source) else { throw ExtractionFailure("손상되었거나 읽을 수 없는 PDF입니다.") }
        guard !document.isEncrypted, !document.isLocked else {
            throw ExtractionFailure("암호화된 PDF는 지원하지 않습니다. 암호화하지 않은 사본을 추가하세요.")
        }
        guard document.pageCount > 0, document.pageCount <= 100 else {
            throw ExtractionFailure("PDF 페이지 수는 1~100이어야 합니다. 자료를 나누어 추가하세요.")
        }
        for index in 0..<document.pageCount {
            let unit: [String: Any] = try autoreleasepool {
                guard let page = document.page(at: index) else {
                    throw ExtractionFailure("PDF \(index + 1)페이지를 읽을 수 없습니다.")
                }
                let image = try raster(page)
                var text = (page.string ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                if text.filter({ !$0.isWhitespace }).count < 32 {
                    let ocr = try recognize(image)
                    if text.isEmpty { text = ocr }
                    else {
                        let additional = ocr.components(separatedBy: .newlines).filter { !text.contains($0) && !$0.isEmpty }
                        if !additional.isEmpty { text += "\n" + additional.joined(separator: "\n") }
                    }
                }
                // PDFPage.string covers page text, not editable note contents.
                // Retain visible text notes even on pages where body text is
                // plentiful and OCR is intentionally skipped. Ink/highlight
                // appearance is represented in the accompanying page image.
                let notes = page.annotations.filter {
                    $0.shouldDisplay && ["FreeText", "Text"].contains($0.type ?? "")
                }.compactMap { $0.contents?.trimmingCharacters(in: .whitespacesAndNewlines) }
                 .filter { !$0.isEmpty }
                if !notes.isEmpty {
                    text += (text.isEmpty ? "" : "\n\n") + "[PDF 메모]\n" + notes.joined(separator: "\n")
                }
                let imageURL = output.appendingPathComponent(String(format: "page-%04d.png", index + 1))
                try encodePNG(image, at: imageURL, totalBytes: &totalBytes)
                return ["locator": ["kind": "page", "index": index + 1], "text": text,
                        "images": [["path": imageURL.path, "mime": "image/png"]]]
            }
            units.append(unit)
        }
        return ["kind": "pdf", "units": units]
    }
    guard ["png", "jpg", "jpeg", "heic", "heif"].contains(ext) else {
        throw ExtractionFailure("이 추출기는 PDF, PNG, JPEG, HEIC만 지원합니다.")
    }
    let image = try decodeImage(source)
    let text = try recognize(image)
    let imageURL = output.appendingPathComponent("image-0001.png")
    try encodePNG(image, at: imageURL, totalBytes: &totalBytes)
    units.append(["locator": ["kind": "image", "index": 1], "text": text,
                  "images": [["path": imageURL.path, "mime": "image/png"]]])
    return ["kind": "image", "units": units]
}

do {
    // `--pages <PDF>` only counts pages, so a run can be planned before extraction.
    if CommandLine.arguments.count == 3, CommandLine.arguments[1] == "--pages" {
        guard let document = PDFDocument(url: URL(fileURLWithPath: CommandLine.arguments[2])), !document.isLocked else {
            throw ExtractionFailure("PDF를 열 수 없습니다.")
        }
        print(document.pageCount)
        exit(0)
    }
    guard CommandLine.arguments.count == 3 else {
        throw ExtractionFailure("사용법: stology-extract <입력 파일> <전용 캐시 폴더>")
    }
    let source = URL(fileURLWithPath: CommandLine.arguments[1]).standardizedFileURL
    let output = URL(fileURLWithPath: CommandLine.arguments[2], isDirectory: true).standardizedFileURL
    let result = try extract(source: source, output: output)
    let json = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys, .withoutEscapingSlashes])
    FileHandle.standardOutput.write(json)
    FileHandle.standardOutput.write(Data([10]))
} catch {
    FileHandle.standardError.write(Data("\(error)\n".utf8))
    exit(1)
}
