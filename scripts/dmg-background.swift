// Draws the background of the Blinker.dmg window:
// assets/macos/dmg-background.png and dmg-background@2x.png.
//
//   swift scripts/dmg-background.swift
//
// The layout matches scripts/dmg-settings.py: a 660×400 window, the app icon
// at (180, 200) and Applications at (480, 200).

import AppKit
import CoreText

let width: CGFloat = 660
let height: CGFloat = 400
let iconY: CGFloat = 200
let appX: CGFloat = 180
let applicationsX: CGFloat = 480

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> NSColor {
    NSColor(srgbRed: CGFloat(hex >> 16 & 0xff) / 255, green: CGFloat(hex >> 8 & 0xff) / 255,
            blue: CGFloat(hex & 0xff) / 255, alpha: alpha)
}

// black, with the brand's snow white glowing under each icon: Finder draws
// the icon names in black on a background picture, in dark mode too
let bg = color(0x0c0c0b)
let snow = color(0xd8d5cd)
let labelY: CGFloat = 283
let text = color(0xe6e4de)
let dim = color(0x9a978e)
let faint = color(0x6e6c66)
let dots = color(0x8a877f)

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
for name in ["FiraSans-Regular", "FiraSans-Medium", "FiraMono-Regular"] {
    let url = root.appendingPathComponent("assets/fonts/\(name).ttf")
    CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
}

func font(_ name: String, _ size: CGFloat) -> NSFont {
    NSFont(name: name, size: size) ?? NSFont.systemFont(ofSize: size)
}

// y from the top of the window, like Finder's icon positions
func centered(_ string: String, _ font: NSFont, _ color: NSColor, y: CGFloat, kern: CGFloat = 0) {
    let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color, .kern: kern]
    let s = NSAttributedString(string: string, attributes: attributes)
    let size = s.size()
    s.draw(at: NSPoint(x: ((width - size.width) / 2).rounded(), y: height - y - size.height / 2))
}

// an oval of snow, its alpha going through the stops from the middle out
func glow(x: CGFloat, y: CGFloat, rx: CGFloat, ry: CGFloat, _ stops: [(CGFloat, CGFloat)]) {
    var locations = stops.map { $0.0 }
    let gradient = NSGradient(colors: stops.map { snow.withAlphaComponent($0.1) },
                              atLocations: &locations, colorSpace: .sRGB)!
    NSGraphicsContext.saveGraphicsState()
    let t = NSAffineTransform()
    t.translateX(by: x, yBy: height - y)
    t.scaleX(by: 1, yBy: ry / rx)
    t.concat()
    gradient.draw(fromCenter: .zero, radius: 0, toCenter: .zero, radius: rx, options: [])
    NSGraphicsContext.restoreGraphicsState()
}

func draw() {
    bg.setFill()
    NSRect(x: 0, y: 0, width: width, height: height).fill()

    for (x, name) in [(appX, "Blinker"), (applicationsX, "Applications")] {
        // a faint aura around the icon, and under the name a snow pill that
        // fits it, the way Finder sizes the name (13 pt system font)
        glow(x: x, y: iconY + 8, rx: 132, ry: 118, [(0, 0.12), (0.45, 0.05), (1, 0)])
        let w = NSAttributedString(string: name, attributes: [.font: NSFont.systemFont(ofSize: 13)]).size().width
        let pill = NSRect(x: x - w / 2 - 12, y: height - labelY - 11, width: w + 24, height: 22)
        NSGraphicsContext.saveGraphicsState()
        let shadow = NSShadow()
        shadow.shadowColor = snow.withAlphaComponent(0.45)
        shadow.shadowBlurRadius = 14
        shadow.set()
        snow.setFill()
        NSBezierPath(roundedRect: pill, xRadius: 11, yRadius: 11).fill()
        NSGraphicsContext.restoreGraphicsState()
    }

    centered("Перетащи Blinker в «Программы»", font("FiraSans-Medium", 21), text, y: 56)
    centered("Drag Blinker to Applications", font("FiraSans-Regular", 14), dim, y: 84)

    // the arrow between the icons
    let from = appX + 86, to = applicationsX - 86
    let y = height - iconY - 6
    let arrow = NSBezierPath()
    arrow.lineWidth = 2.5
    arrow.lineCapStyle = .round
    arrow.lineJoinStyle = .round
    arrow.setLineDash([0.1, 9], count: 2, phase: 0)
    arrow.move(to: NSPoint(x: from, y: y))
    arrow.line(to: NSPoint(x: to - 6, y: y))
    dots.setStroke()
    arrow.stroke()
    let head = NSBezierPath()
    head.lineWidth = 2.5
    head.lineCapStyle = .round
    head.lineJoinStyle = .round
    head.move(to: NSPoint(x: to - 10, y: y + 10))
    head.line(to: NSPoint(x: to, y: y))
    head.line(to: NSPoint(x: to - 10, y: y - 10))
    dim.setStroke()
    head.stroke()

    // the first launch, since the app is not notarized
    centered("Не открывается? Настройки → Конфиденциальность и безопасность → Всё равно открыть",
             font("FiraSans-Regular", 12), dim, y: 350)
    centered("Won’t open? System Settings → Privacy & Security → Open Anyway",
             font("FiraSans-Regular", 12), faint, y: 369)
    centered("blinker.cool", font("FiraMono-Regular", 11), faint, y: 388, kern: 0.5)
}

for scale in [1, 2] {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(width) * scale,
                               pixelsHigh: Int(height) * scale, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                               bytesPerRow: 0, bitsPerPixel: 0)!
    rep.size = NSSize(width: width, height: height)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    draw()
    NSGraphicsContext.restoreGraphicsState()
    let name = scale == 1 ? "dmg-background.png" : "dmg-background@2x.png"
    let url = root.appendingPathComponent("assets/macos/\(name)")
    try! rep.representation(using: .png, properties: [:])!.write(to: url)
    print("wrote \(url.path)")
}
