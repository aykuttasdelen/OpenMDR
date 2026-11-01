import Foundation
import PDFKit
import Vision
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

// MARK: - Data Structures
struct OCRProgress: Codable {
    let type: String
    let page: Int
    let totalPages: Int
    let percent: Int
}

struct OCRPageResult: Codable {
    let pageNumber: Int
    let isOcr: Bool
    let text: String
    let images: [String]
}

struct OCRFinalResult: Codable {
    let type: String
    let success: Bool
    let totalPages: Int
    let processedPages: Int
    let markdown: String
    let pages: [OCRPageResult]
    let extractedImages: [String]
    let error: String?
}

struct RecognizedLine {
    let text: String
    let boundingBox: CGRect // Vision coords: (0,0) is bottom-left
    let top: CGFloat        // Top-down coords: (0,0) is top-left
    let bottom: CGFloat
    let height: CGFloat
    let left: CGFloat
    let right: CGFloat
}

struct ExtractedAsset {
    let relativePath: String
    let fullPath: String
    let top: CGFloat        // 0.0 = top of page, 1.0 = bottom
    let bottom: CGFloat
    let rect: CGRect
}

// MARK: - Helper Functions
func encodeJSON<T: Encodable>(_ value: T) -> String? {
    let encoder = JSONEncoder()
    if #available(macOS 13.0, *) {
        encoder.outputFormatting = [.withoutEscapingSlashes]
    }
    guard let data = try? encoder.encode(value) else { return nil }
    return String(data: data, encoding: .utf8)
}

func emitProgress(page: Int, total: Int) {
    let percent = total > 0 ? Int((Double(page) / Double(total)) * 100) : 100
    let progress = OCRProgress(type: "progress", page: page, totalPages: total, percent: percent)
    if let str = encodeJSON(progress) {
        print(str)
        fflush(stdout)
    }
}

func emitError(_ message: String) {
    let result = OCRFinalResult(
        type: "result",
        success: false,
        totalPages: 0,
        processedPages: 0,
        markdown: "",
        pages: [],
        extractedImages: [],
        error: message
    )
    if let str = encodeJSON(result) {
        print(str)
        fflush(stdout)
    }
}

func emitResult(_ result: OCRFinalResult) {
    if let str = encodeJSON(result) {
        print(str)
        fflush(stdout)
    }
}

// Render PDF Page to CGImage at high DPI for Vision OCR
func renderPageToCGImage(page: PDFPage, dpi: CGFloat = 300.0) -> CGImage? {
    let bounds = page.bounds(for: .cropBox)
    guard bounds.width > 0 && bounds.height > 0 else { return nil }
    
    let rot = page.rotation
    let isLandscape = (rot == 90 || rot == 270)
    let pageW = isLandscape ? bounds.height : bounds.width
    let pageH = isLandscape ? bounds.width : bounds.height
    
    let scale = dpi / 72.0
    let width = max(1, Int(ceil(pageW * scale)))
    let height = max(1, Int(ceil(pageH * scale)))
    
    let colorSpace = CGColorSpaceCreateDeviceRGB()
    guard let context = CGContext(
        data: nil,
        width: width,
        height: height,
        bitsPerComponent: 8,
        bytesPerRow: 0,
        space: colorSpace,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else {
        return nil
    }
    
    // Fill with white background
    context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: width, height: height))
    
    context.saveGState()
    context.scaleBy(x: scale, y: scale)
    context.concatenate(page.transform(for: .cropBox))
    page.draw(with: .cropBox, to: context)
    context.restoreGState()
    
    return context.makeImage()
}

// Render a specific sub-rect of a PDFPage to CGImage in raw PDF user space
func renderRectFromPage(page: PDFPage, rect: CGRect, dpi: CGFloat = 200.0) -> CGImage? {
    guard rect.width > 0 && rect.height > 0 else { return nil }
    let scale = dpi / 72.0
    let width = max(1, Int(ceil(rect.width * scale)))
    let height = max(1, Int(ceil(rect.height * scale)))
    
    let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
    guard let context = CGContext(
        data: nil,
        width: width,
        height: height,
        bitsPerComponent: 8,
        bytesPerRow: 0,
        space: colorSpace,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else {
        return nil
    }
    
    context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: width, height: height))
    
    context.saveGState()
    context.scaleBy(x: scale, y: scale)
    context.translateBy(x: -rect.origin.x, y: -rect.origin.y)
    if let cgPage = page.pageRef {
        context.drawPDFPage(cgPage)
    } else {
        page.draw(with: .mediaBox, to: context)
    }
    context.restoreGState()
    
    return context.makeImage()
}

// Save CGImage as normalized 8-bit sRGB PNG atomically
func saveCGImageAsPNG(_ image: CGImage, to fileUrl: URL) -> Bool {
    let width = image.width
    let height = image.height
    guard width > 0 && height > 0 else { return false }

    let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
    var targetImage = image

    if let ctx = CGContext(
        data: nil,
        width: width,
        height: height,
        bitsPerComponent: 8,
        bytesPerRow: 0,
        space: colorSpace,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) {
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        if let normalized = ctx.makeImage() {
            targetImage = normalized
        }
    }

    let outData = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(
        outData as CFMutableData,
        UTType.png.identifier as CFString,
        1,
        nil
    ) else {
        return false
    }
    CGImageDestinationAddImage(destination, targetImage, nil)
    guard CGImageDestinationFinalize(destination), outData.length > 0 else {
        return false
    }
    return (try? (outData as Data).write(to: fileUrl, options: .atomic)) != nil
}

// Sanitize document/subfolder names to ASCII-safe identifiers (handling Turkish NFD/NFC characters)
func sanitizeDocName(_ raw: String) -> String {
    let turkishMap: [Character: String] = [
        "ç": "c", "Ç": "C",
        "ğ": "g", "Ğ": "G",
        "ı": "i", "I": "I",
        "İ": "I", "i": "i",
        "ö": "o", "Ö": "O",
        "ş": "s", "Ş": "S",
        "ü": "u", "Ü": "U"
    ]
    let precomposed = raw.precomposedStringWithCanonicalMapping
    var mapped = ""
    for ch in precomposed {
        if let replacement = turkishMap[ch] {
            mapped.append(replacement)
        } else {
            mapped.append(ch)
        }
    }
    let folded = mapped.folding(options: [.diacriticInsensitive, .widthInsensitive], locale: Locale(identifier: "en_US_POSIX"))
    let asciiAllowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789")
    let cleaned = folded
        .components(separatedBy: asciiAllowed.inverted)
        .filter { !$0.isEmpty }
        .joined(separator: "_")
    return cleaned.isEmpty ? "asset" : cleaned
}

// MARK: - PDF Content & Image Extraction

// Inspect stream dictionary to extract original JPEG data or decode CGImage
func extractImageFromStream(_ stream: CGPDFStreamRef, allowRawFallback: Bool = true) -> (cgImage: CGImage?, jpegData: Data?, ext: String)? {
    guard let streamDict = CGPDFStreamGetDictionary(stream) else { return nil }
    
    var width: CGPDFInteger = 0
    var height: CGPDFInteger = 0
    guard CGPDFDictionaryGetInteger(streamDict, "Width", &width),
          CGPDFDictionaryGetInteger(streamDict, "Height", &height),
          width > 0 && height > 0 else {
        return nil
    }
    
    // Check if the image stream carries an external alpha mask or custom Decode array;
    // when positioned on a page (allowRawFallback == false), prefer page rendering so mask & decode are composited.
    var maskObj: CGPDFObjectRef?
    let hasMask = CGPDFDictionaryGetObject(streamDict, "SMask", &maskObj) || CGPDFDictionaryGetObject(streamDict, "Mask", &maskObj)
    var decodeArr: CGPDFArrayRef?
    let hasDecode = CGPDFDictionaryGetArray(streamDict, "Decode", &decodeArr)
    if !allowRawFallback && (hasMask || hasDecode) {
        return nil
    }
    
    var bpc: CGPDFInteger = 8
    CGPDFDictionaryGetInteger(streamDict, "BitsPerComponent", &bpc)
    
    var format: CGPDFDataFormat = .raw
    guard let cfData = CGPDFStreamCopyData(stream, &format) else { return nil }
    let data = cfData as Data
    guard !data.isEmpty else { return nil }
    
    // Check if JPEG encoded and verify it can be decoded into a valid CGImage
    if format.rawValue == 1 || (data.count > 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF) {
        if let source = CGImageSourceCreateWithData(data as CFData, nil),
           let img = CGImageSourceCreateImageAtIndex(source, 0, nil) {
            if (img.colorSpace?.model == .rgb || img.colorSpace?.model == .monochrome) && !hasMask && !hasDecode {
                return (nil, data, "jpg")
            } else {
                // Convert CMYK or non-standard color space JPEGs to normalized sRGB PNG
                return (img, nil, "png")
            }
        }
    }
    
    // Check if standard image data readable by ImageIO (PNG, JPEG 2000, TIFF, GIF)
    if let source = CGImageSourceCreateWithData(data as CFData, nil),
       let img = CGImageSourceCreateImageAtIndex(source, 0, nil) {
        return (img, nil, "png")
    }
    
    // Raw bitmap data reconstruction
    var components = 3
    var colorSpace: CGColorSpace = CGColorSpaceCreateDeviceRGB()
    var isUnsupportedRawColorSpace = false
    
    var csObj: CGPDFObjectRef?
    if CGPDFDictionaryGetObject(streamDict, "ColorSpace", &csObj), let obj = csObj {
        var namePtr: UnsafePointer<CChar>?
        if CGPDFObjectGetValue(obj, .name, &namePtr), let np = namePtr {
            let name = String(cString: np)
            if name == "DeviceGray" {
                colorSpace = CGColorSpaceCreateDeviceGray()
                components = 1
            } else if name == "DeviceCMYK" {
                colorSpace = CGColorSpaceCreateDeviceCMYK()
                components = 4
            } else if name == "DeviceRGB" {
                colorSpace = CGColorSpaceCreateDeviceRGB()
                components = 3
            } else {
                isUnsupportedRawColorSpace = true
            }
        } else {
            var arr: CGPDFArrayRef?
            if CGPDFObjectGetValue(obj, .array, &arr), let a = arr {
                var firstItemName: UnsafePointer<CChar>?
                if CGPDFArrayGetName(a, 0, &firstItemName), let fn = firstItemName {
                    let csFamily = String(cString: fn)
                    if csFamily == "DeviceGray" || csFamily == "CalGray" {
                        colorSpace = CGColorSpaceCreateDeviceGray()
                        components = 1
                    } else if csFamily == "DeviceCMYK" {
                        colorSpace = CGColorSpaceCreateDeviceCMYK()
                        components = 4
                    } else if csFamily == "DeviceRGB" || csFamily == "CalRGB" {
                        colorSpace = CGColorSpaceCreateDeviceRGB()
                        components = 3
                    } else if csFamily == "ICCBased" {
                        var iccStream: CGPDFStreamRef?
                        if CGPDFArrayGetStream(a, 1, &iccStream), let isRef = iccStream,
                           let isDict = CGPDFStreamGetDictionary(isRef) {
                            var nComp: CGPDFInteger = 3
                            if CGPDFDictionaryGetInteger(isDict, "N", &nComp) {
                                if nComp == 1 {
                                    colorSpace = CGColorSpaceCreateDeviceGray()
                                    components = 1
                                } else if nComp == 4 {
                                    colorSpace = CGColorSpaceCreateDeviceCMYK()
                                    components = 4
                                } else {
                                    colorSpace = CGColorSpaceCreateDeviceRGB()
                                    components = 3
                                }
                            }
                        }
                    } else {
                        // Indexed, Separation, DeviceN, Lab, Pattern require PDFKit rendering
                        isUnsupportedRawColorSpace = true
                    }
                }
            }
        }
    }
    
    if isUnsupportedRawColorSpace && !allowRawFallback {
        return nil
    }
    
    // Infer components from data size if not explicitly gray or cmyk
    let pixelCount = Int(width) * Int(height)
    if pixelCount > 0 && !isUnsupportedRawColorSpace {
        if data.count >= pixelCount * 4 && components != 1 {
            components = 4
            if colorSpace.numberOfComponents != 4 {
                colorSpace = CGColorSpaceCreateDeviceCMYK()
            }
        } else if data.count >= pixelCount * 3 && components != 1 {
            components = 3
            if colorSpace.numberOfComponents != 3 {
                colorSpace = CGColorSpaceCreateDeviceRGB()
            }
        } else if data.count == pixelCount {
            components = 1
            colorSpace = CGColorSpaceCreateDeviceGray()
        }
    }
    
    let bytesPerRow: Int
    if bpc == 1 {
        bytesPerRow = (Int(width) * components + 7) / 8
    } else if bpc < 8 {
        bytesPerRow = (Int(width) * components * Int(bpc) + 7) / 8
    } else {
        bytesPerRow = Int(width) * components * max(1, Int(bpc) / 8)
    }
    
    if data.count >= bytesPerRow * Int(height),
       let provider = CGDataProvider(data: data as CFData) {
        let bitmapInfo = CGBitmapInfo(rawValue: CGImageAlphaInfo.none.rawValue)
        if let img = CGImage(
            width: Int(width),
            height: Int(height),
            bitsPerComponent: Int(bpc),
            bitsPerPixel: Int(bpc) * components,
            bytesPerRow: bytesPerRow,
            space: colorSpace,
            bitmapInfo: bitmapInfo,
            provider: provider,
            decode: nil,
            shouldInterpolate: true,
            intent: .defaultIntent
        ) {
            return (img, nil, "png")
        }
    }
    
    return nil
}

// Scanner state for tracking CTM and Do operators
class PageScannerState {
    var ctmStack: [CGAffineTransform] = [.identity]
    var currentCTM: CGAffineTransform = .identity
    var invocations: [(name: String, rect: CGRect, ctm: CGAffineTransform)] = []
}

func getInheritedResources(from dict: CGPDFDictionaryRef) -> CGPDFDictionaryRef? {
    var resources: CGPDFDictionaryRef?
    if CGPDFDictionaryGetDictionary(dict, "Resources", &resources) {
        return resources
    }
    var parent: CGPDFDictionaryRef?
    if CGPDFDictionaryGetDictionary(dict, "Parent", &parent), let p = parent {
        return getInheritedResources(from: p)
    }
    return nil
}

func scanContentStream(contentStream: CGPDFContentStreamRef, state: PageScannerState) {
    guard let table = CGPDFOperatorTableCreate() else { return }
    
    CGPDFOperatorTableSetCallback(table, "q") { scanner, info in
        let s = Unmanaged<PageScannerState>.fromOpaque(info!).takeUnretainedValue()
        s.ctmStack.append(s.currentCTM)
    }
    CGPDFOperatorTableSetCallback(table, "Q") { scanner, info in
        let s = Unmanaged<PageScannerState>.fromOpaque(info!).takeUnretainedValue()
        if !s.ctmStack.isEmpty {
            s.currentCTM = s.ctmStack.removeLast()
        }
    }
    CGPDFOperatorTableSetCallback(table, "cm") { scanner, info in
        let s = Unmanaged<PageScannerState>.fromOpaque(info!).takeUnretainedValue()
        var a: CGPDFReal = 0, b: CGPDFReal = 0, c: CGPDFReal = 0, d: CGPDFReal = 0, tx: CGPDFReal = 0, ty: CGPDFReal = 0
        guard CGPDFScannerPopNumber(scanner, &ty),
              CGPDFScannerPopNumber(scanner, &tx),
              CGPDFScannerPopNumber(scanner, &d),
              CGPDFScannerPopNumber(scanner, &c),
              CGPDFScannerPopNumber(scanner, &b),
              CGPDFScannerPopNumber(scanner, &a) else { return }
        let t = CGAffineTransform(a: a, b: b, c: c, d: d, tx: tx, ty: ty)
        s.currentCTM = t.concatenating(s.currentCTM)
    }
    CGPDFOperatorTableSetCallback(table, "Do") { scanner, info in
        let s = Unmanaged<PageScannerState>.fromOpaque(info!).takeUnretainedValue()
        var namePtr: UnsafePointer<CChar>?
        if CGPDFScannerPopName(scanner, &namePtr), let np = namePtr {
            let name = String(cString: np)
            let unitRect = CGRect(x: 0, y: 0, width: 1, height: 1)
            let rect = unitRect.applying(s.currentCTM).standardized
            s.invocations.append((name: name, rect: rect, ctm: s.currentCTM))
        }
    }
    
    let scanner = CGPDFScannerCreate(contentStream, table, Unmanaged.passUnretained(state).toOpaque())
    CGPDFScannerScan(scanner)
    CGPDFScannerRelease(scanner)
    CGPDFOperatorTableRelease(table)
}

struct DiscoveredImage {
    let name: String
    let rect: CGRect
    let stream: CGPDFStreamRef?
}

func scanPageRecursively(cgPage: CGPDFPage) -> [DiscoveredImage] {
    guard let pageDict = cgPage.dictionary else { return [] }
    let pageBounds = cgPage.getBoxRect(.mediaBox)
    guard pageBounds.width > 0 && pageBounds.height > 0 else { return [] }
    
    let initialResources = getInheritedResources(from: pageDict)
    let pageStream = CGPDFContentStreamCreateWithPage(cgPage)
    let pageState = PageScannerState()
    scanContentStream(contentStream: pageStream, state: pageState)
    
    var results: [DiscoveredImage] = []
    
    func resolveXObject(name: String, inRes: CGPDFDictionaryRef?) -> (stream: CGPDFStreamRef?, subtype: String, formResources: CGPDFDictionaryRef?, matrix: CGAffineTransform)? {
        guard let res = inRes ?? initialResources else { return nil }
        var xo: CGPDFDictionaryRef?
        guard CGPDFDictionaryGetDictionary(res, "XObject", &xo), let xDict = xo else { return nil }
        var stream: CGPDFStreamRef?
        guard CGPDFDictionaryGetStream(xDict, name, &stream), let s = stream else { return nil }
        guard let sDict = CGPDFStreamGetDictionary(s) else { return nil }
        var subtypePtr: UnsafePointer<CChar>?
        guard CGPDFDictionaryGetName(sDict, "Subtype", &subtypePtr), let st = subtypePtr else { return nil }
        let subtype = String(cString: st)
        var fRes: CGPDFDictionaryRef?
        CGPDFDictionaryGetDictionary(sDict, "Resources", &fRes)
        
        var matrix = CGAffineTransform.identity
        var matrixArr: CGPDFArrayRef?
        if CGPDFDictionaryGetArray(sDict, "Matrix", &matrixArr), let ma = matrixArr, CGPDFArrayGetCount(ma) == 6 {
            var a: CGPDFReal = 0, b: CGPDFReal = 0, c: CGPDFReal = 0, d: CGPDFReal = 0, tx: CGPDFReal = 0, ty: CGPDFReal = 0
            CGPDFArrayGetNumber(ma, 0, &a)
            CGPDFArrayGetNumber(ma, 1, &b)
            CGPDFArrayGetNumber(ma, 2, &c)
            CGPDFArrayGetNumber(ma, 3, &d)
            CGPDFArrayGetNumber(ma, 4, &tx)
            CGPDFArrayGetNumber(ma, 5, &ty)
            matrix = CGAffineTransform(a: a, b: b, c: c, d: d, tx: tx, ty: ty)
        }
        return (s, subtype, fRes, matrix)
    }
    
    func processInvocations(_ invs: [(name: String, rect: CGRect, ctm: CGAffineTransform)], parentStream: CGPDFContentStreamRef, currentRes: CGPDFDictionaryRef?, depth: Int) {
        if depth > 5 { return }
        for inv in invs {
            if let resolved = resolveXObject(name: inv.name, inRes: currentRes) {
                if resolved.subtype == "Image" {
                    results.append(DiscoveredImage(name: inv.name, rect: inv.rect, stream: resolved.stream))
                } else if resolved.subtype == "Form", let formStreamRef = resolved.stream {
                    guard let formRes = resolved.formResources ?? currentRes ?? initialResources else { continue }
                    let formContentStream = CGPDFContentStreamCreateWithStream(formStreamRef, formRes, parentStream)
                    let nestedState = PageScannerState()
                    let formCTM = resolved.matrix.concatenating(inv.ctm)
                    nestedState.currentCTM = formCTM
                    scanContentStream(contentStream: formContentStream, state: nestedState)
                    processInvocations(nestedState.invocations, parentStream: formContentStream, currentRes: formRes, depth: depth + 1)
                }
            }
        }
    }
    
    processInvocations(pageState.invocations, parentStream: pageStream, currentRes: initialResources, depth: 0)
    return results
}

func extractImagesFromPage(
    page: PDFPage,
    pageNumber: Int,
    baseName: String,
    assetsDir: URL,
    subfolderName: String,
    savedPaths: inout Set<String>
) -> [ExtractedAsset] {
    guard let cgPage = page.pageRef,
           let pageDict = cgPage.dictionary else {
        return []
    }
    
    let pageBounds = page.bounds(for: .mediaBox)
    guard pageBounds.width > 0 && pageBounds.height > 0 else { return [] }
    
    var assets: [ExtractedAsset] = []
    var imageIndex = 1
    
    func nextUniqueAssetUrl(ext: String) -> (url: URL, relativePath: String) {
        var candidateName = "\(baseName)-p\(pageNumber)-\(imageIndex).\(ext)"
        var counter = 1
        var candidateUrl = assetsDir.appendingPathComponent(candidateName)
        while FileManager.default.fileExists(atPath: candidateUrl.path) || savedPaths.contains(candidateUrl.path) {
            candidateName = "\(baseName)-p\(pageNumber)-\(imageIndex)-\(counter).\(ext)"
            candidateUrl = assetsDir.appendingPathComponent(candidateName)
            counter += 1
        }
        let rel = subfolderName.isEmpty ? "assets/\(candidateName)" : "assets/\(subfolderName)/\(candidateName)"
        return (candidateUrl, rel)
    }

    func commitSavedAsset(url: URL) {
        imageIndex += 1
        savedPaths.insert(url.path)
    }
    
    let discovered = scanPageRecursively(cgPage: cgPage)
    var processedStreams = Set<UInt>()
    var processedNames = Set<String>()
    
    func markMaskStreams(of stream: CGPDFStreamRef?) {
        guard let s = stream, let sDict = CGPDFStreamGetDictionary(s) else { return }
        for key in ["SMask", "Mask"] {
            var maskStream: CGPDFStreamRef?
            if CGPDFDictionaryGetStream(sDict, key, &maskStream), let ms = maskStream {
                processedStreams.insert(unsafeBitCast(ms, to: UInt.self))
            }
        }
    }
    
    // 1. Process images discovered via content stream (with rects)
    for item in discovered {
        guard item.rect.width >= 4 && item.rect.height >= 4 else { continue }
        markMaskStreams(of: item.stream)
        
        let normTop = max(0.0, min(1.0, 1.0 - (item.rect.origin.y + item.rect.size.height) / pageBounds.height))
        let normBottom = max(0.0, min(1.0, 1.0 - item.rect.origin.y / pageBounds.height))
        
        var savedSuccessfully = false
        
        if let s = item.stream, let extracted = extractImageFromStream(s, allowRawFallback: false) {
            let (targetUrl, relPath) = nextUniqueAssetUrl(ext: extracted.ext)
            if let data = extracted.jpegData {
                if (try? data.write(to: targetUrl, options: .atomic)) != nil {
                    commitSavedAsset(url: targetUrl)
                    assets.append(ExtractedAsset(
                        relativePath: relPath,
                        fullPath: targetUrl.path,
                        top: normTop,
                        bottom: normBottom,
                        rect: item.rect
                    ))
                    savedSuccessfully = true
                }
            } else if let img = extracted.cgImage {
                if saveCGImageAsPNG(img, to: targetUrl) {
                    commitSavedAsset(url: targetUrl)
                    assets.append(ExtractedAsset(
                        relativePath: relPath,
                        fullPath: targetUrl.path,
                        top: normTop,
                        bottom: normBottom,
                        rect: item.rect
                    ))
                    savedSuccessfully = true
                }
            }
        }
        
        // Fallback: render rect directly from page (handles Indexed, SMask, Decode, Lab, Separation)
        if !savedSuccessfully {
            if let rendered = renderRectFromPage(page: page, rect: item.rect) {
                let (targetUrl, relPath) = nextUniqueAssetUrl(ext: "png")
                if saveCGImageAsPNG(rendered, to: targetUrl) {
                    commitSavedAsset(url: targetUrl)
                    assets.append(ExtractedAsset(
                        relativePath: relPath,
                        fullPath: targetUrl.path,
                        top: normTop,
                        bottom: normBottom,
                        rect: item.rect
                    ))
                    savedSuccessfully = true
                }
            }
        }
        
        if savedSuccessfully {
            if let s = item.stream {
                processedStreams.insert(unsafeBitCast(s, to: UInt.self))
            }
            processedNames.insert(item.name)
        }
    }
    
    // 2. Also check any XObjects in resources hierarchy not captured by stream scanner
    func collectAllResourceImages(from dict: CGPDFDictionaryRef?, into list: inout [(name: String, stream: CGPDFStreamRef)], visited: inout Set<UInt>) {
        guard let d = dict else { return }
        let dId = unsafeBitCast(d, to: UInt.self)
        if visited.contains(dId) { return }
        visited.insert(dId)
        
        var xo: CGPDFDictionaryRef?
        if CGPDFDictionaryGetDictionary(d, "XObject", &xo), let xDict = xo {
            CGPDFDictionaryApplyBlock(xDict, { key, val, _ in
                let name = String(cString: key)
                var stream: CGPDFStreamRef?
                if CGPDFObjectGetValue(val, .stream, &stream), let s = stream, let sDict = CGPDFStreamGetDictionary(s) {
                    var subtypePtr: UnsafePointer<CChar>?
                    if CGPDFDictionaryGetName(sDict, "Subtype", &subtypePtr), let st = subtypePtr {
                        let subtype = String(cString: st)
                        if subtype == "Image" {
                            markMaskStreams(of: s)
                            list.append((name: name, stream: s))
                        } else if subtype == "Form" {
                            var fRes: CGPDFDictionaryRef?
                            if CGPDFDictionaryGetDictionary(sDict, "Resources", &fRes) {
                                collectAllResourceImages(from: fRes, into: &list, visited: &visited)
                            }
                        }
                    }
                }
                return true
            }, nil)
        }
    }
    
    var allResImages: [(name: String, stream: CGPDFStreamRef)] = []
    var visitedDicts = Set<UInt>()
    let pageResources = getInheritedResources(from: pageDict)
    collectAllResourceImages(from: pageResources, into: &allResImages, visited: &visitedDicts)
    
    for resImg in allResImages {
        let sId = unsafeBitCast(resImg.stream, to: UInt.self)
        if processedStreams.contains(sId) || processedNames.contains(resImg.name) {
            continue
        }
        
        if let extracted = extractImageFromStream(resImg.stream, allowRawFallback: true) {
            let (targetUrl, relPath) = nextUniqueAssetUrl(ext: extracted.ext)
            var didSave = false
            if let data = extracted.jpegData {
                didSave = (try? data.write(to: targetUrl, options: .atomic)) != nil
            } else if let img = extracted.cgImage {
                didSave = saveCGImageAsPNG(img, to: targetUrl)
            }
            if didSave {
                commitSavedAsset(url: targetUrl)
                assets.append(ExtractedAsset(
                    relativePath: relPath,
                    fullPath: targetUrl.path,
                    top: 0.5,
                    bottom: 0.5,
                    rect: .zero
                ))
                processedStreams.insert(sId)
                processedNames.insert(resImg.name)
            }
        }
    }
    
    return assets
}

// MARK: - OCR & Text Extraction

// Perform Vision OCR on CGImage and interleave extracted image assets
func performOCR(
    on image: CGImage,
    languages: [String],
    extractedAssets: [ExtractedAsset] = []
) -> (markdown: String, rawText: String) {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    
    if !languages.isEmpty {
        if #available(macOS 12.0, *) {
            if let supported = try? request.supportedRecognitionLanguages() {
                let filtered = languages.filter { supported.contains($0) }
                if !filtered.isEmpty {
                    request.recognitionLanguages = filtered
                }
            } else {
                request.recognitionLanguages = languages
            }
        } else {
            request.recognitionLanguages = languages
        }
    }
    
    let handler = VNImageRequestHandler(cgImage: image, options: [:])
    do {
        try handler.perform([request])
    } catch {
        if !extractedAssets.isEmpty {
            let imgMd = extractedAssets.map { "![Görsel](\($0.relativePath))" }.joined(separator: "\n\n")
            return (imgMd, "")
        }
        return ("", "")
    }
    
    guard let observations = request.results, !observations.isEmpty else {
        if !extractedAssets.isEmpty {
            let imgMd = extractedAssets.map { "![Görsel](\($0.relativePath))" }.joined(separator: "\n\n")
            return (imgMd, "")
        }
        return ("", "")
    }
    
    // Extract recognized lines with normalized coordinates
    var lines: [RecognizedLine] = []
    for obs in observations {
        guard let candidate = obs.topCandidates(1).first else { continue }
        let text = candidate.string.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { continue }
        
        let bb = obs.boundingBox
        // Convert bottom-left (Vision) to top-left coordinates
        let top = 1.0 - (bb.origin.y + bb.size.height)
        let bottom = 1.0 - bb.origin.y
        let left = bb.origin.x
        let right = bb.origin.x + bb.size.width
        let height = bb.size.height
        
        lines.append(RecognizedLine(
            text: text,
            boundingBox: bb,
            top: top,
            bottom: bottom,
            height: height,
            left: left,
            right: right
        ))
    }
    
    guard !lines.isEmpty else {
        if !extractedAssets.isEmpty {
            let imgMd = extractedAssets.map { "![Görsel](\($0.relativePath))" }.joined(separator: "\n\n")
            return (imgMd, "")
        }
        return ("", "")
    }
    
    // Sort lines top to bottom, grouping horizontally adjacent items
    lines.sort { (a, b) -> Bool in
        let aCenterY = (a.top + a.bottom) / 2.0
        let bCenterY = (b.top + b.bottom) / 2.0
        let tolerance = min(a.height, b.height) * 0.4
        
        if abs(aCenterY - bCenterY) < tolerance {
            return a.left < b.left
        }
        return a.top < b.top
    }
    
    // Calculate average line height to detect headings
    let totalHeight = lines.reduce(0.0) { $0 + $1.height }
    let avgHeight = totalHeight / CGFloat(lines.count)
    
    var mdBlocks: [String] = []
    var currentParagraph: [String] = []
    var prevLine: RecognizedLine? = nil
    
    func flushParagraph() {
        if !currentParagraph.isEmpty {
            mdBlocks.append(currentParagraph.joined(separator: " "))
            currentParagraph.removeAll()
        }
    }
    
    var remainingAssets = extractedAssets.sorted { $0.top < $1.top }
    
    func flushAssets(beforeTop targetTop: CGFloat) {
        while let nextAsset = remainingAssets.first, nextAsset.top <= targetTop {
            flushParagraph()
            mdBlocks.append("![Görsel](\(nextAsset.relativePath))")
            remainingAssets.removeFirst()
        }
    }
    
    for line in lines {
        flushAssets(beforeTop: line.top)
        
        let isHeading1 = line.height > avgHeight * 1.8
        let isHeading2 = !isHeading1 && line.height > avgHeight * 1.35
        
        var trimmed = line.text
        
        // Bullet points normalization
        let bulletPrefixes = ["•", "▪", "▫", "‣", "–", "—"]
        var isBullet = false
        for bp in bulletPrefixes {
            if trimmed.hasPrefix(bp) {
                trimmed = "- " + trimmed.dropFirst(bp.count).trimmingCharacters(in: .whitespaces)
                isBullet = true
                break
            }
        }
        if !isBullet && (trimmed.hasPrefix("- ") || trimmed.hasPrefix("* ")) {
            isBullet = true
        }
        
        // Ordered list check (e.g. "1. " or "1) ")
        let isOrderedList = trimmed.range(of: #"^\d+[\.\)]\s+"#, options: .regularExpression) != nil
        
        // Check vertical gap from previous line
        let hasLargeGap: Bool
        if let prev = prevLine {
            let gap = line.top - prev.bottom
            hasLargeGap = gap > avgHeight * 1.2
        } else {
            hasLargeGap = false
        }
        
        if isHeading1 {
            flushParagraph()
            mdBlocks.append("# " + trimmed.replacingOccurrences(of: "^#+\\s*", with: "", options: .regularExpression))
        } else if isHeading2 {
            flushParagraph()
            mdBlocks.append("## " + trimmed.replacingOccurrences(of: "^#+\\s*", with: "", options: .regularExpression))
        } else if isBullet || isOrderedList {
            flushParagraph()
            mdBlocks.append(trimmed)
        } else if hasLargeGap {
            flushParagraph()
            currentParagraph.append(trimmed)
        } else {
            currentParagraph.append(trimmed)
        }
        
        prevLine = line
    }
    
    flushParagraph()
    
    // Remaining assets below text
    while !remainingAssets.isEmpty {
        let asset = remainingAssets.removeFirst()
        mdBlocks.append("![Görsel](\(asset.relativePath))")
    }
    
    let markdown = mdBlocks.joined(separator: "\n\n")
    let rawText = lines.map { $0.text }.joined(separator: "\n")
    return (markdown, rawText)
}

// Extract digital text directly from PDFPage if present and interleave extracted image assets
func extractDigitalText(
    from page: PDFPage,
    extractedAssets: [ExtractedAsset] = []
) -> (markdown: String, rawText: String)? {
    guard let pageString = page.string?.trimmingCharacters(in: .whitespacesAndNewlines),
          pageString.count > 30 else {
        return nil
    }
    
    // Format lines intelligently
    let rawLines = pageString.components(separatedBy: .newlines)
    var cleanedBlocks: [String] = []
    var currentGroup: [String] = []
    
    for raw in rawLines {
        let trimmed = raw.trimmingCharacters(in: .whitespaces)
        if trimmed.isEmpty {
            if !currentGroup.isEmpty {
                cleanedBlocks.append(currentGroup.joined(separator: " "))
                currentGroup.removeAll()
            }
        } else {
            currentGroup.append(trimmed)
        }
    }
    if !currentGroup.isEmpty {
        cleanedBlocks.append(currentGroup.joined(separator: " "))
    }
    
    // Insert extracted assets into digital text according to their normalized vertical position
    if !extractedAssets.isEmpty {
        let sortedAssets = extractedAssets.sorted { $0.top < $1.top }
        let initialCount = cleanedBlocks.count
        var offset = 0
        for asset in sortedAssets {
            let targetIdx: Int
            if initialCount == 0 {
                targetIdx = 0
            } else if asset.top <= 0.15 {
                targetIdx = 0
            } else if asset.top >= 0.85 {
                targetIdx = initialCount
            } else {
                let ratio = Double(asset.top)
                targetIdx = min(initialCount, max(0, Int(round(ratio * Double(initialCount)))))
            }
            cleanedBlocks.insert("![Görsel](\(asset.relativePath))", at: min(cleanedBlocks.count, targetIdx + offset))
            offset += 1
        }
    }
    
    let markdown = cleanedBlocks.joined(separator: "\n\n")
    return (markdown, pageString)
}

// MARK: - Main Execution
func main() {
    let args = CommandLine.arguments
    var inputPath: String?
    var outputDirPath: String?
    var languages: [String] = ["tr-TR", "en-US"]
    var pageSelection: String = "all"
    var mode: String = "hybrid" // "hybrid", "ocr", "text"
    var extractImages: Bool = true
    var customSubfolder: String?
    
    var i = 1
    while i < args.count {
        switch args[i] {
        case "--input", "-i":
            if i + 1 < args.count {
                inputPath = args[i + 1]
                i += 1
            }
        case "--output-dir", "-o":
            if i + 1 < args.count {
                outputDirPath = args[i + 1]
                i += 1
            }
        case "--languages", "-l":
            if i + 1 < args.count {
                languages = args[i + 1].components(separatedBy: ",").map { $0.trimmingCharacters(in: .whitespaces) }
                i += 1
            }
        case "--pages", "-p":
            if i + 1 < args.count {
                pageSelection = args[i + 1]
                i += 1
            }
        case "--mode", "-m":
            if i + 1 < args.count {
                mode = args[i + 1]
                i += 1
            }
        case "--extract-images":
            if i + 1 < args.count {
                let val = args[i + 1].lowercased()
                extractImages = (val == "true" || val == "1" || val == "yes")
                i += 1
            }
        case "--subfolder":
            if i + 1 < args.count {
                customSubfolder = args[i + 1]
                i += 1
            }
        default:
            break
        }
        i += 1
    }
    
    guard let path = inputPath, !path.isEmpty else {
        emitError("Missing required --input argument.")
        exit(1)
    }
    
    let fileUrl = URL(fileURLWithPath: path)
    guard FileManager.default.fileExists(atPath: fileUrl.path) else {
        emitError("PDF file does not exist at path: \(path)")
        exit(1)
    }
    
    guard let pdfDocument = PDFDocument(url: fileUrl) else {
        emitError("Failed to open PDF document. It may be corrupted or password-protected.")
        exit(1)
    }
    
    let totalCount = pdfDocument.pageCount
    guard totalCount > 0 else {
        emitError("PDF document contains no pages.")
        exit(1)
    }
    
    // Resolve output assets directory
    let destinationDirUrl: URL
    if let outPath = outputDirPath, !outPath.isEmpty {
        destinationDirUrl = URL(fileURLWithPath: outPath)
    } else {
        destinationDirUrl = fileUrl.deletingLastPathComponent()
    }
    let assetsDirUrl = destinationDirUrl.appendingPathComponent("assets")
    
    let rawBaseName = fileUrl.deletingPathExtension().lastPathComponent
    let baseDocName = sanitizeDocName(rawBaseName)
    
    let subfolderName = customSubfolder.map { sanitizeDocName($0) } ?? baseDocName
    let docAssetsDirUrl = subfolderName.isEmpty ? assetsDirUrl : assetsDirUrl.appendingPathComponent(subfolderName)
    
    if extractImages {
        try? FileManager.default.createDirectory(at: docAssetsDirUrl, withIntermediateDirectories: true)
    }
    
    // Parse target pages (1-indexed for CLI, 0-indexed for PDFDocument)
    var targetIndices: [Int] = []
    if pageSelection.lowercased() == "all" {
        targetIndices = Array(0..<totalCount)
    } else {
        let parts = pageSelection.components(separatedBy: ",")
        for part in parts {
            let trimmed = part.trimmingCharacters(in: .whitespaces)
            if trimmed.contains("-") {
                let rangeParts = trimmed.components(separatedBy: "-")
                if rangeParts.count == 2,
                   let start = Int(rangeParts[0].trimmingCharacters(in: .whitespaces)),
                   let end = Int(rangeParts[1].trimmingCharacters(in: .whitespaces)) {
                    let minP = max(1, start)
                    let maxP = min(totalCount, end)
                    if minP <= maxP {
                        for p in minP...maxP {
                            targetIndices.append(p - 1)
                        }
                    }
                }
            } else if let p = Int(trimmed), p >= 1 && p <= totalCount {
                targetIndices.append(p - 1)
            }
        }
        targetIndices = Array(Set(targetIndices)).sorted()
    }
    
    if targetIndices.isEmpty {
        targetIndices = Array(0..<totalCount)
    }
    
    var results: [OCRPageResult] = []
    var fullMarkdownParts: [String] = []
    var allExtractedAssetPaths: [String] = []
    var allSavedAssetDiskPaths = Set<String>()
    
    for (stepIndex, pageIdx) in targetIndices.enumerated() {
        guard let page = pdfDocument.page(at: pageIdx) else { continue }
        let pageNumber = pageIdx + 1
        
        emitProgress(page: stepIndex + 1, total: targetIndices.count)
        
        // 1. Extract any images inside this page
        var pageAssets: [ExtractedAsset] = []
        if extractImages {
            pageAssets = extractImagesFromPage(
                page: page,
                pageNumber: pageNumber,
                baseName: baseDocName,
                assetsDir: docAssetsDirUrl,
                subfolderName: subfolderName,
                savedPaths: &allSavedAssetDiskPaths
            )
        }
        
        for asset in pageAssets {
            allExtractedAssetPaths.append(asset.relativePath)
        }
        
        var pageMarkdown = ""
        var isOcr = false
        
        if mode == "text" {
            if let digital = extractDigitalText(from: page, extractedAssets: pageAssets) {
                pageMarkdown = digital.markdown
            } else if !pageAssets.isEmpty {
                pageMarkdown = pageAssets.map { "![Görsel](\($0.relativePath))" }.joined(separator: "\n\n")
            }
        } else if mode == "ocr" {
            isOcr = true
            if let image = renderPageToCGImage(page: page) {
                let (md, _) = performOCR(on: image, languages: languages, extractedAssets: pageAssets)
                pageMarkdown = md
            } else if !pageAssets.isEmpty {
                pageMarkdown = pageAssets.map { "![Görsel](\($0.relativePath))" }.joined(separator: "\n\n")
            }
        } else {
            // Hybrid mode: check if page has digital text
            if let digital = extractDigitalText(from: page, extractedAssets: pageAssets) {
                pageMarkdown = digital.markdown
                isOcr = false
            } else {
                // Scanned or image page
                isOcr = true
                if let image = renderPageToCGImage(page: page) {
                    let (md, _) = performOCR(on: image, languages: languages, extractedAssets: pageAssets)
                    pageMarkdown = md
                } else if !pageAssets.isEmpty {
                    pageMarkdown = pageAssets.map { "![Görsel](\($0.relativePath))" }.joined(separator: "\n\n")
                }
            }
        }
        
        let header = targetIndices.count > 1 ? "<!-- Page \(pageNumber) -->\n" : ""
        let formattedPart = header + pageMarkdown
        fullMarkdownParts.append(formattedPart)
        
        results.append(OCRPageResult(
            pageNumber: pageNumber,
            isOcr: isOcr,
            text: pageMarkdown,
            images: pageAssets.map { $0.relativePath }
        ))
    }
    
    let combinedMarkdown = fullMarkdownParts.joined(separator: "\n\n---\n\n")
    
    let finalResult = OCRFinalResult(
        type: "result",
        success: true,
        totalPages: totalCount,
        processedPages: results.count,
        markdown: combinedMarkdown,
        pages: results,
        extractedImages: allExtractedAssetPaths,
        error: nil
    )
    
    emitResult(finalResult)
}

main()
