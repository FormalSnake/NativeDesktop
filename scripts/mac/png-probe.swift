// Reads a PNG back as colour, for the capture assertions in the Chrome style
// gates. Run it with `swift scripts/mac/png-probe.swift <png> [x y w h]`,
// where the rectangle is in image pixels and defaults to the whole image.
//
// Prints one JSON object: the image size, the rectangle's mean colour, and the
// mean colour of sixteen horizontal bands across it. A band that does not
// match the rest is a strip the compositor left stale, which is what a
// live-resize check is looking for.
import CoreGraphics
import Foundation
import ImageIO

let args = CommandLine.arguments
guard args.count > 1,
      let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: args[1]) as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
else {
    FileHandle.standardError.write("png-probe: cannot read an image from \(args.dropFirst().first ?? "")\n".data(using: .utf8)!)
    exit(1)
}

let width = image.width
let height = image.height
var rect = CGRect(x: 0, y: 0, width: width, height: height)
if args.count >= 6, let x = Double(args[2]), let y = Double(args[3]), let w = Double(args[4]), let h = Double(args[5]) {
    rect = CGRect(x: x, y: y, width: w, height: h).intersection(CGRect(x: 0, y: 0, width: width, height: height))
}
guard rect.width >= 1, rect.height >= 1 else {
    FileHandle.standardError.write("png-probe: the rectangle lies outside the \(width)x\(height) image\n".data(using: .utf8)!)
    exit(1)
}

// Redrawn into a known 8-bit RGBA buffer: a captured PNG can arrive in any
// colour space or bit depth and the byte layout has to be predictable here.
let w = Int(rect.width)
let h = Int(rect.height)
var pixels = [UInt8](repeating: 0, count: w * h * 4)
guard let context = pixels.withUnsafeMutableBytes({ bytes in
    CGContext(
        data: bytes.baseAddress, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
}) else {
    FileHandle.standardError.write("png-probe: no bitmap context\n".data(using: .utf8)!)
    exit(1)
}
context.draw(image, in: CGRect(x: -rect.minX, y: rect.minY + rect.height - CGFloat(height), width: CGFloat(width), height: CGFloat(height)))

func mean(rows: Range<Int>) -> [Int] {
    var total = (r: 0, g: 0, b: 0)
    var count = 0
    for row in rows {
        for column in 0..<w {
            let index = (row * w + column) * 4
            total.r += Int(pixels[index])
            total.g += Int(pixels[index + 1])
            total.b += Int(pixels[index + 2])
            count += 1
        }
    }
    guard count > 0 else { return [0, 0, 0] }
    return [total.r / count, total.g / count, total.b / count]
}

let bandCount = min(16, h)
var bands: [[Int]] = []
for band in 0..<bandCount {
    let start = band * h / bandCount
    let end = max(start + 1, (band + 1) * h / bandCount)
    bands.append(mean(rows: start..<min(end, h)))
}

let report: [String: Any] = [
    "width": width,
    "height": height,
    "rect": ["x": Int(rect.minX), "y": Int(rect.minY), "w": w, "h": h],
    "mean": mean(rows: 0..<h),
    "bands": bands,
]
print(String(decoding: try JSONSerialization.data(withJSONObject: report), as: UTF8.self))
