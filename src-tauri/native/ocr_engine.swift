import Foundation
import PDFKit
import Vision

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
}

struct OCRFinalResult: Codable {
    let type: String
    let success: Bool
    let totalPages: Int
    let processedPages: Int
    let markdown: String
    let pages: [OCRPageResult]
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

// MARK: - Helper Functions
func emitProgress(page: Int, total: Int) {
    let percent = total > 0 ? Int((Double(page) / Double(total)) * 100) : 100
    let progress = OCRProgress(type: "progress", page: page, totalPages: total, percent: percent)
    if let data = try? JSONEncoder().encode(progress),
       let str = String(data: data, encoding: .utf8) {
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
        error: message
    )
    if let data = try? JSONEncoder().encode(result),
       let str = String(data: data, encoding: .utf8) {
        print(str)
        fflush(stdout)
    }
}

func emitResult(_ result: OCRFinalResult) {
    if let data = try? JSONEncoder().encode(result),
       let str = String(data: data, encoding: .utf8) {
        print(str)
        fflush(stdout)
    }
}

// Render PDF Page to CGImage at high DPI for Vision OCR
func renderPageToCGImage(page: PDFPage, dpi: CGFloat = 300.0) -> CGImage? {
    let bounds = page.bounds(for: .mediaBox)
    guard bounds.width > 0 && bounds.height > 0 else { return nil }
    
    let scale = dpi / 72.0
    let width = Int(bounds.width * scale)
    let height = Int(bounds.height * scale)
    
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
    page.draw(with: .mediaBox, to: context)
    context.restoreGState()
    
    return context.makeImage()
}

// Perform Vision OCR on CGImage
func performOCR(on image: CGImage, languages: [String]) -> (markdown: String, rawText: String) {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    
    if !languages.isEmpty {
        // Supported languages check
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
        return ("", "")
    }
    
    guard let observations = request.results, !observations.isEmpty else {
        return ("", "")
    }
    
    // Extract recognized lines with normalized coordinates
    var lines: [RecognizedLine] = []
    for obs in observations {
        guard let candidate = obs.topCandidates(1).first else { continue }
        let text = candidate.string.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { continue }
        
        let bb = obs.boundingBox
        // Convert bottom-left (Vision) to top-left (screen) coordinates
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
    
    guard !lines.isEmpty else { return ("", "") }
    
    // Sort lines top to bottom, grouping horizontally adjacent items
    lines.sort { (a, b) -> Bool in
        // If vertical center difference is small, sort by X coordinate
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
    
    for line in lines {
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
        if !isBullet && trimmed.hasPrefix("- ") || trimmed.hasPrefix("* ") {
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
    
    let markdown = mdBlocks.joined(separator: "\n\n")
    let rawText = lines.map { $0.text }.joined(separator: "\n")
    return (markdown, rawText)
}

// Extract digital text directly from PDFPage if present
func extractDigitalText(from page: PDFPage) -> (markdown: String, rawText: String)? {
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
    
    let markdown = cleanedBlocks.joined(separator: "\n\n")
    return (markdown, pageString)
}

// MARK: - Main Execution
func main() {
    let args = CommandLine.arguments
    var inputPath: String?
    var languages: [String] = ["tr-TR", "en-US"]
    var pageSelection: String = "all"
    var mode: String = "hybrid" // "hybrid", "ocr", "text"
    
    var i = 1
    while i < args.count {
        switch args[i] {
        case "--input", "-i":
            if i + 1 < args.count {
                inputPath = args[i + 1]
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
    
    for (stepIndex, pageIdx) in targetIndices.enumerated() {
        guard let page = pdfDocument.page(at: pageIdx) else { continue }
        let pageNumber = pageIdx + 1
        
        emitProgress(page: stepIndex + 1, total: targetIndices.count)
        
        var pageMarkdown = ""
        var isOcr = false
        
        if mode == "text" {
            if let digital = extractDigitalText(from: page) {
                pageMarkdown = digital.markdown
            }
        } else if mode == "ocr" {
            isOcr = true
            if let image = renderPageToCGImage(page: page) {
                let (md, _) = performOCR(on: image, languages: languages)
                pageMarkdown = md
            }
        } else {
            // Hybrid mode: check if page has digital text
            if let digital = extractDigitalText(from: page) {
                pageMarkdown = digital.markdown
                isOcr = false
            } else {
                // Scanned or image page
                isOcr = true
                if let image = renderPageToCGImage(page: page) {
                    let (md, _) = performOCR(on: image, languages: languages)
                    pageMarkdown = md
                }
            }
        }
        
        let header = targetIndices.count > 1 ? "<!-- Page \(pageNumber) -->\n" : ""
        let formattedPart = header + pageMarkdown
        fullMarkdownParts.append(formattedPart)
        
        results.append(OCRPageResult(
            pageNumber: pageNumber,
            isOcr: isOcr,
            text: pageMarkdown
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
        error: nil
    )
    
    emitResult(finalResult)
}

main()
