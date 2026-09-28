// Measures where a text field's content sits inside its bezel on a capture,
// for the vertical-centring assertion in scripts/headerfield-drive.ts.
//
//   swift scripts/mac/field-ink.swift <png> <x> <y> <w> <h> <iconStart> <split> <textEnd>
//
// The rectangle (image pixels) is the field's frame from getTree. Columns
// from `iconStart` to `split` hold the leading glyph (padlock or magnifier),
// clear of the capsule's rounded end; `split` to `textEnd` hold the text, and
// the columns past `textEnd` give each row's fill. Prints one JSON object,
// all values image rows:
//
//   bezel  the frame's first and last row
//   icon   the leading glyph's ink rows
//   text   ink top, baseline (the lowest dense row, so descenders do not pull
//          the centre down) and ink bottom
import CoreGraphics
import Foundation
import ImageIO

let args = CommandLine.arguments
guard args.count >= 9,
      let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: args[1]) as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
      let rx = Int(args[2]), let ry = Int(args[3]), let rw = Int(args[4]), let rh = Int(args[5]),
      let iconStart = Int(args[6]), let split = Int(args[7]), let textEnd = Int(args[8])
else {
    FileHandle.standardError.write("usage: field-ink.swift <png> <x> <y> <w> <h> <iconStart> <split> <textEnd>\n".data(using: .utf8)!)
    exit(1)
}

let width = image.width
let height = image.height
var pixels = [UInt8](repeating: 0, count: width * height * 4)
guard let context = pixels.withUnsafeMutableBytes({ bytes in
    CGContext(
        data: bytes.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
}) else { exit(1) }
context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))

func px(_ x: Int, _ y: Int) -> (Int, Int, Int) {
    let i = (y * width + x) * 4
    return (Int(pixels[i]), Int(pixels[i + 1]), Int(pixels[i + 2]))
}
func diff(_ a: (Int, Int, Int), _ b: (Int, Int, Int)) -> Int {
    abs(a.0 - b.0) + abs(a.1 - b.1) + abs(a.2 - b.2)
}
func fail(_ m: String) -> Never {
    FileHandle.standardError.write("field-ink: \(m)\n".data(using: .utf8)!)
    exit(1)
}

let x0 = max(0, rx), x1 = min(width, rx + rw)
let y0 = max(0, ry), y1 = min(height, ry + rh)
guard x1 > x0, y1 > y0, iconStart >= x0, split > iconStart, textEnd > split, textEnd <= x1 else { fail("rectangle out of range") }

// The capsule is the field's frame: the toolbar's platter view has the same
// frame as the field it holds, and on a light toolbar its edge is too faint to
// find on the capture. Its fill shades from top to bottom, so each row's own
// fill is the commonest colour on that row past the text, where only the
// capsule (and at most the cancel glyph) is drawn.
let bezelTop = y0, bezelBottom = y1 - 1
let probeCols = Array(stride(from: textEnd, to: x1, by: 2))
guard probeCols.count >= 4 else { fail("no room past the text to read the fill") }
func rowFill(_ y: Int) -> (Int, Int, Int) {
    var counts: [Int: Int] = [:]
    for x in probeCols {
        let (r, g, b) = px(x, y)
        counts[(r >> 2) << 16 | (g >> 2) << 8 | (b >> 2), default: 0] += 1
    }
    let key = counts.max { $0.value < $1.value }!.key
    return (((key >> 16) & 0xff) << 2, ((key >> 8) & 0xff) << 2, (key & 0xff) << 2)
}

/// Ink per row between two columns, inside the bezel minus its rim.
func inkProfile(_ from: Int, _ to: Int) -> [(row: Int, count: Int)] {
    // A twelfth of the capsule's height at each edge is its rim and shading;
    // no glyph reaches it.
    let rim = (bezelBottom - bezelTop) / 12
    return ((bezelTop + rim)...(bezelBottom - rim)).map { y in
        let fill = rowFill(y)
        return (y, (from..<to).reduce(0) { $0 + (diff(px($1, y), fill) > 90 ? 1 : 0) })
    }
}

func inkSpan(_ profile: [(row: Int, count: Int)]) -> (Int, Int)? {
    let rows = profile.filter { $0.count > 0 }.map(\.row)
    guard let t = rows.first, let b = rows.last else { return nil }
    return (t, b)
}

let icon = inkSpan(inkProfile(iconStart, split))
let textProfile = inkProfile(split, textEnd)
guard let text = inkSpan(textProfile) else { fail("no text ink in the field") }
// Baseline: the lowest row carrying at least 40% of the densest row's ink.
let peak = textProfile.map(\.count).max() ?? 0
let baseline = textProfile.filter { $0.count * 10 >= peak * 4 }.map(\.row).max() ?? text.1

func json(_ v: (Int, Int)?) -> String { v.map { "[\($0.0),\($0.1)]" } ?? "null" }
print("{\"size\":[\(width),\(height)],\"bezel\":[\(bezelTop),\(bezelBottom)],\"icon\":\(json(icon)),\"text\":[\(text.0),\(baseline),\(text.1)]}")
