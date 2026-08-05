import AppKit
import CoreText
import Metal
import QuartzCore
import simd

// GPU text for the editor canvas.
//
// Why: with 17 tabs and 3 panes, the packaged -O build spends 240 ms of a
// ~280 ms face budget inside `EditorCanvasView.draw`, and 85% of THAT is
// CoreText — `CTLineDraw` 103 ms, shaping 83 ms, gutter numbers 19.5 ms
// (measured, see docs/SUISEI-GPU-ARCHITECTURE.md §4.3). Worst-case single draw
// was 21.9 ms: two and a half 120 Hz frames. Meanwhile the machine's ten GPU
// cores do nothing but blit the bitmap the CPU just rasterized.
//
// The shape of the fix is standard for monospaced text: rasterize each glyph
// ONCE into an atlas, then draw the screen as instanced quads that sample it.
// Per frame the CPU writes a flat instance buffer and issues a couple of draw
// calls; no shaping, no rasterization, no per-line CGContext work.
//
// Scope of THIS file: the device, the atlas, and the two pipelines (rects and
// glyphs). It is deliberately independent of `EditorCanvasView` so it can be
// developed and measured against the CoreText path rather than replacing it —
// see `SUISEI_RENDERER`.

/// Which renderer the editor canvas uses. CoreText stays the default until the
/// Metal path is proven on the same measurements that justified it.
enum RendererChoice {
    static let useMetal = ProcessInfo.processInfo.environment["SUISEI_RENDERER"] == "metal"
}

// MARK: - Instance layouts
//
// These MUST match the structs in `shaderSource` below, field for field.

/// One background/decoration quad: cursor line, selection, find wash, git
/// stripe, caret, underline. 32 bytes.
struct RectInstance {
    var rect: SIMD4<Float>      // x, y, w, h in points
    var color: SIMD4<Float>     // premultiplied RGBA
}

/// One glyph. 32 bytes — kept wide enough for exact positions rather than a
/// cell grid, because Suisei must render Korean and CJK, whose advances do not
/// land on the Latin cell pitch.
struct GlyphInstance {
    var pos: SIMD2<Float>       // top-left in points
    var size: SIMD2<Float>      // quad size in points
    var uv0: SIMD2<Float>       // atlas top-left, normalised
    var uv1: SIMD2<Float>       // atlas bottom-right, normalised
    var color: SIMD4<Float>
}

struct Uniforms {
    /// Multiply a point coordinate by this and add `origin` to get clip space.
    var scale: SIMD2<Float>
    var origin: SIMD2<Float>
    /// Scroll offset in points, applied in the vertex shader. This is why a
    /// scroll costs nothing: nothing is re-emitted, one float2 changes.
    var scroll: SIMD2<Float>
    var _pad: SIMD2<Float>
}

// MARK: - Glyph atlas

/// A key identifies a rasterized glyph exactly: the same character in a
/// different font (CJK fallback) or size is a different entry.
struct GlyphKey: Hashable {
    let font: CTFont
    let glyph: CGGlyph
    /// Rounded to 1/4 pt so a zoom step does not fragment the atlas.
    let sizeQ: Int
    /// Backing scale — a Retina glyph is a different bitmap.
    let scaleQ: Int

    static func == (a: GlyphKey, b: GlyphKey) -> Bool {
        a.glyph == b.glyph && a.sizeQ == b.sizeQ && a.scaleQ == b.scaleQ
            && CFEqual(a.font, b.font)
    }

    func hash(into h: inout Hasher) {
        h.combine(glyph)
        h.combine(sizeQ)
        h.combine(scaleQ)
        h.combine(CFHash(font))
    }
}

/// Where a glyph lives in the atlas, and how to place it.
struct GlyphSlot {
    var uv0: SIMD2<Float>
    var uv1: SIMD2<Float>
    /// Size of the quad in POINTS (bitmap pixels / scale).
    var size: SIMD2<Float>
    /// Offset from the pen position to the quad's top-left, in points.
    var bearing: SIMD2<Float>
}

/// Single-channel coverage atlas, shelf-packed.
///
/// `.r8Unorm` is not a shortcut: macOS has used grayscale antialiasing since
/// Mojave, so one channel is exactly what the platform renders. It is also a
/// third of the memory of an RGB subpixel atlas and needs no per-channel blend.
///
/// Colour glyphs (emoji, `sbix`/`COLR` fonts) are NOT handled here — they need
/// a BGRA atlas and a second pass. The renderer falls back to CoreText for any
/// run containing one, so nothing renders wrong in the meantime.
final class GlyphAtlas {
    static let dimension = 2048

    private(set) var texture: MTLTexture
    private var slots: [GlyphKey: GlyphSlot] = [:]
    /// Shelf packing: current row's origin and the tallest glyph placed in it.
    private var penX = 1
    private var penY = 1
    private var shelfHeight = 0
    private var full = false

    /// Scratch bitmap reused for every rasterization — one allocation, not one
    /// per glyph.
    private var scratch: CGContext?
    private var scratchW = 0
    private var scratchH = 0

    init?(device: MTLDevice) {
        let desc = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: .r8Unorm,
            width: Self.dimension,
            height: Self.dimension,
            mipmapped: false
        )
        desc.usage = .shaderRead
        desc.storageMode = .shared
        guard let tex = device.makeTexture(descriptor: desc) else { return nil }
        texture = tex
        // Clear once: an uninitialised atlas shows as garbage speckle behind
        // glyphs whose bounds round outward.
        let zero = [UInt8](repeating: 0, count: Self.dimension * Self.dimension)
        zero.withUnsafeBytes { raw in
            texture.replace(
                region: MTLRegionMake2D(0, 0, Self.dimension, Self.dimension),
                mipmapLevel: 0,
                withBytes: raw.baseAddress!,
                bytesPerRow: Self.dimension
            )
        }
    }

    /// Slot for a glyph, rasterizing it on first sight. Nil when the glyph has
    /// no coverage (space) or the atlas is out of room.
    func slot(for key: GlyphKey, scale: CGFloat) -> GlyphSlot? {
        if let existing = slots[key] { return existing }
        guard !full else { return nil }
        guard let s = rasterize(key, scale: scale) else { return nil }
        slots[key] = s
        return s
    }

    /// True when this glyph is already resident — lets the caller decide to
    /// fall back rather than stall a frame on rasterization.
    func isResident(_ key: GlyphKey) -> Bool { slots[key] != nil }

    private func rasterize(_ key: GlyphKey, scale: CGFloat) -> GlyphSlot? {
        var glyph = key.glyph
        var bounds = CGRect.zero
        CTFontGetBoundingRectsForGlyphs(key.font, .horizontal, &glyph, &bounds, 1)
        guard bounds.width > 0, bounds.height > 0 else {
            // No ink (space, control). Record an empty slot so we do not retry
            // it on every frame.
            let empty = GlyphSlot(
                uv0: .zero, uv1: .zero, size: .zero, bearing: .zero
            )
            slots[key] = empty
            return empty
        }

        // Pad by one pixel each side: CoreText antialiasing bleeds past the
        // typographic bounds, and a tight box clips the softest edge row.
        let pad: CGFloat = 1
        let pxW = Int(ceil((bounds.width + pad * 2) * scale))
        let pxH = Int(ceil((bounds.height + pad * 2) * scale))
        guard pxW > 0, pxH > 0, pxW < Self.dimension, pxH < Self.dimension else {
            return nil
        }

        guard let ctx = scratchContext(width: pxW, height: pxH) else { return nil }

        // The scratch buffer is REUSED and grows to the largest glyph seen, so
        // it is usually bigger than this glyph. That makes the vertical origin
        // load-bearing: a CGBitmapContext's memory row 0 is the TOP of the
        // image, while its coordinate system counts y UP from the bottom. So
        // drawing at y≈0 puts the ink in the LAST rows of memory — and the
        // upload below reads the FIRST pxH rows, which are blank. Everything
        // reported "resident" and the screen stayed empty.
        //
        // Place the padded box against the top of the buffer instead, which is
        // exactly the region uploaded.
        let topY = CGFloat(scratchH - pxH)
        ctx.setFillColor(gray: 0, alpha: 1)
        ctx.fill(CGRect(x: 0, y: topY, width: CGFloat(pxW), height: CGFloat(pxH)))
        ctx.setFillColor(gray: 1, alpha: 1)
        ctx.setShouldAntialias(true)
        ctx.setShouldSmoothFonts(false)   // grayscale AA, matching the platform

        var origin = CGPoint(x: pad - bounds.minX, y: pad - bounds.minY)
        ctx.saveGState()
        ctx.translateBy(x: 0, y: topY)
        ctx.scaleBy(x: scale, y: scale)
        CTFontDrawGlyphs(key.font, &glyph, &origin, 1, ctx)
        ctx.restoreGState()

        guard let placed = allocate(width: pxW, height: pxH) else {
            full = true
            return nil
        }

        guard let data = ctx.data else { return nil }
        let bytesPerRow = ctx.bytesPerRow
        // The scratch context is a single-channel bitmap, so its rows map
        // straight into the atlas.
        texture.replace(
            region: MTLRegionMake2D(placed.x, placed.y, pxW, pxH),
            mipmapLevel: 0,
            withBytes: data,
            bytesPerRow: bytesPerRow
        )

        let dim = Float(Self.dimension)
        // No V flip. The uploaded rows already run top-down (see the origin
        // note above), so atlas row `placed.y` IS the glyph's top edge, and V
        // increases downward the same way the quad's corner does.
        return GlyphSlot(
            uv0: SIMD2(Float(placed.x) / dim, Float(placed.y) / dim),
            uv1: SIMD2(Float(placed.x + pxW) / dim, Float(placed.y + pxH) / dim),
            size: SIMD2(Float(CGFloat(pxW) / scale), Float(CGFloat(pxH) / scale)),
            bearing: SIMD2(
                Float(bounds.minX - pad),
                // Distance from the baseline UP to the quad's top edge.
                Float(bounds.maxY + pad)
            )
        )
    }

    private func scratchContext(width: Int, height: Int) -> CGContext? {
        if let ctx = scratch, scratchW >= width, scratchH >= height { return ctx }
        let w = max(width, scratchW, 128)
        let h = max(height, scratchH, 128)
        guard let space = CGColorSpace(name: CGColorSpace.linearGray),
              let ctx = CGContext(
                data: nil, width: w, height: h,
                bitsPerComponent: 8, bytesPerRow: w,
                space: space, bitmapInfo: CGImageAlphaInfo.none.rawValue
              )
        else { return nil }
        scratch = ctx
        scratchW = w
        scratchH = h
        return ctx
    }

    private func allocate(width: Int, height: Int) -> (x: Int, y: Int)? {
        if penX + width + 1 > Self.dimension {
            // Next shelf.
            penX = 1
            penY += shelfHeight + 1
            shelfHeight = 0
        }
        guard penY + height + 1 <= Self.dimension else { return nil }
        let spot = (x: penX, y: penY)
        penX += width + 1
        shelfHeight = max(shelfHeight, height)
        return spot
    }
}

// MARK: - Shaders
//
// Compiled at runtime with `makeLibrary(source:)` rather than shipped as a
// `.metal` file, because the packaging script is a flat `swiftc` invocation
// with an explicit source list — adding a metal compile+link step to it is a
// build-system change this does not need yet. Cost is a one-off ~30 ms at
// first use; `MTLBinaryArchive` can remove even that later.
//
// Neither pipeline binds a vertex buffer. The quad comes from `vertex_id` and
// the instance is read straight out of a device pointer, which is the Apple-GPU
// idiom and skips input assembly entirely.
private let shaderSource = """
#include <metal_stdlib>
using namespace metal;

struct Uniforms {
    float2 scale;
    float2 origin;
    float2 scroll;
    float2 _pad;
};

struct RectInstance {
    float4 rect;
    float4 color;
};

struct GlyphInstance {
    float2 pos;
    float2 size;
    float2 uv0;
    float2 uv1;
    float4 color;
};

struct VOut {
    float4 position [[position]];
    float2 uv;
    float4 color;
};

// Unit-quad corner from the vertex id: 0,0  1,0  0,1  1,1 (triangle strip).
static inline float2 corner(uint vid) {
    return float2(float(vid & 1u), float((vid >> 1) & 1u));
}

// Points (y-down, origin top-left) -> clip space (y-up, centre origin).
//
// x:  0 -> -1,  W -> +1        =>  x * (2/W) - 1
// y:  0 -> +1,  H -> -1        =>  1 - y * (2/H)
//
// The y term SUBTRACTS from origin. Writing it as `(p * scale + origin)` and
// negating the result afterwards is not the same thing — it maps y=0 to -1 and
// pushes the whole viewport outside the frustum, which renders exactly nothing.
static inline float4 toClip(float2 p, constant Uniforms &u) {
    float2 q = p + u.scroll;
    return float4(q.x * u.scale.x + u.origin.x,
                  u.origin.y - q.y * u.scale.y,
                  0.0, 1.0);
}

vertex VOut rect_vs(uint vid [[vertex_id]],
                    uint iid [[instance_id]],
                    device const RectInstance *inst [[buffer(0)]],
                    constant Uniforms &u [[buffer(1)]]) {
    RectInstance r = inst[iid];
    float2 c = corner(vid);
    float2 p = r.rect.xy + c * r.rect.zw;
    VOut o;
    o.position = toClip(p, u);
    o.uv = c;
    o.color = r.color;
    return o;
}

fragment half4 rect_fs(VOut in [[stage_in]]) {
    return half4(in.color);
}

vertex VOut glyph_vs(uint vid [[vertex_id]],
                     uint iid [[instance_id]],
                     device const GlyphInstance *inst [[buffer(0)]],
                     constant Uniforms &u [[buffer(1)]]) {
    GlyphInstance g = inst[iid];
    float2 c = corner(vid);
    float2 p = g.pos + c * g.size;
    VOut o;
    o.position = toClip(p, u);
    o.uv = mix(g.uv0, g.uv1, c);
    o.color = g.color;
    return o;
}

fragment half4 glyph_fs(VOut in [[stage_in]],
                        texture2d<half> atlas [[texture(0)]],
                        sampler samp [[sampler(0)]]) {
    // Single-channel coverage. Premultiply so the blend below is a plain
    // source-over and the text edges stay gamma-correct against any background.
    half a = atlas.sample(samp, in.uv).r * half(in.color.a);
    return half4(half3(in.color.rgb) * a, a);
}
"""

// MARK: - Renderer

/// Owns the device, the pipelines and the per-frame instance buffers.
///
/// One instance per canvas. The atlas is shared process-wide for the same
/// reason the CoreText caches are (see `EditorCanvasView`): panes show the same
/// document, and a per-pane atlas would rasterize every glyph once per pane.
final class MetalTextRenderer {
    static let device: MTLDevice? = MTLCreateSystemDefaultDevice()
    nonisolated(unsafe) private static var sharedAtlas: GlyphAtlas?

    let device: MTLDevice
    let queue: MTLCommandQueue
    let atlas: GlyphAtlas
    private let rectPipeline: MTLRenderPipelineState
    private let glyphPipeline: MTLRenderPipelineState
    private let sampler: MTLSamplerState

    /// Triple-buffered instance storage, so the CPU can build frame N+1 while
    /// the GPU still reads frame N.
    private var rectBuffers: [MTLBuffer] = []
    private var glyphBuffers: [MTLBuffer] = []
    private var frameIndex = 0
    private let inFlight = DispatchSemaphore(value: 3)

    private var rects: [RectInstance] = []
    private var glyphs: [GlyphInstance] = []

    init?() {
        guard let dev = Self.device, let q = dev.makeCommandQueue() else { return nil }
        device = dev
        queue = q

        if let existing = Self.sharedAtlas {
            atlas = existing
        } else {
            guard let a = GlyphAtlas(device: dev) else { return nil }
            Self.sharedAtlas = a
            atlas = a
        }

        guard let library = try? dev.makeLibrary(source: shaderSource, options: nil),
              let rectVS = library.makeFunction(name: "rect_vs"),
              let rectFS = library.makeFunction(name: "rect_fs"),
              let glyphVS = library.makeFunction(name: "glyph_vs"),
              let glyphFS = library.makeFunction(name: "glyph_fs")
        else { return nil }

        func pipeline(_ vs: MTLFunction, _ fs: MTLFunction) -> MTLRenderPipelineState? {
            let d = MTLRenderPipelineDescriptor()
            d.vertexFunction = vs
            d.fragmentFunction = fs
            let att = d.colorAttachments[0]!
            att.pixelFormat = .bgra8Unorm
            // Premultiplied source-over: both passes emit premultiplied colour.
            att.isBlendingEnabled = true
            att.rgbBlendOperation = .add
            att.alphaBlendOperation = .add
            att.sourceRGBBlendFactor = .one
            att.sourceAlphaBlendFactor = .one
            att.destinationRGBBlendFactor = .oneMinusSourceAlpha
            att.destinationAlphaBlendFactor = .oneMinusSourceAlpha
            return try? dev.makeRenderPipelineState(descriptor: d)
        }
        guard let rp = pipeline(rectVS, rectFS), let gp = pipeline(glyphVS, glyphFS) else {
            return nil
        }
        rectPipeline = rp
        glyphPipeline = gp

        let sd = MTLSamplerDescriptor()
        // Linear: glyph quads are placed at fractional positions for CJK, so
        // nearest would shimmer as text scrolls.
        sd.minFilter = .linear
        sd.magFilter = .linear
        sd.sAddressMode = .clampToEdge
        sd.tAddressMode = .clampToEdge
        guard let s = dev.makeSamplerState(descriptor: sd) else { return nil }
        sampler = s

        for _ in 0..<3 {
            guard let rb = dev.makeBuffer(length: 64 * 1024, options: .storageModeShared),
                  let gb = dev.makeBuffer(length: 512 * 1024, options: .storageModeShared)
            else { return nil }
            rectBuffers.append(rb)
            glyphBuffers.append(gb)
        }
    }

    // MARK: Frame building

    func beginFrame() {
        rects.removeAll(keepingCapacity: true)
        glyphs.removeAll(keepingCapacity: true)
    }

    func addRect(_ r: CGRect, _ color: NSColor) {
        guard let c = color.usingColorSpace(.sRGB) else { return }
        let a = Float(c.alphaComponent)
        rects.append(RectInstance(
            rect: SIMD4(Float(r.minX), Float(r.minY), Float(r.width), Float(r.height)),
            // Premultiplied, to match the blend state.
            color: SIMD4(
                Float(c.redComponent) * a,
                Float(c.greenComponent) * a,
                Float(c.blueComponent) * a,
                a
            )
        ))
    }

    /// Append one shaped run at `origin` (baseline-left, in points).
    /// Returns false when any glyph is not resident and could not be added —
    /// the caller should fall back rather than draw a partial line.
    @discardableResult
    func addGlyphs(
        font: CTFont,
        glyphs runGlyphs: [CGGlyph],
        positions: [CGPoint],
        origin: CGPoint,
        color: NSColor,
        scale: CGFloat
    ) -> Bool {
        guard runGlyphs.count == positions.count else { return false }
        guard let c = color.usingColorSpace(.sRGB) else { return false }
        let rgba = SIMD4(
            Float(c.redComponent), Float(c.greenComponent),
            Float(c.blueComponent), Float(c.alphaComponent)
        )
        let sizeQ = Int((CTFontGetSize(font) * 4).rounded())
        let scaleQ = Int((scale * 4).rounded())
        for i in 0..<runGlyphs.count {
            let key = GlyphKey(
                font: font, glyph: runGlyphs[i], sizeQ: sizeQ, scaleQ: scaleQ
            )
            guard let slot = atlas.slot(for: key, scale: scale) else { return false }
            if slot.size.x == 0 { continue }   // no ink
            let penX = origin.x + positions[i].x
            let penY = origin.y + positions[i].y
            glyphs.append(GlyphInstance(
                pos: SIMD2(
                    Float(penX) + slot.bearing.x,
                    Float(penY) - slot.bearing.y
                ),
                size: slot.size,
                uv0: slot.uv0,
                uv1: slot.uv1,
                color: rgba
            ))
        }
        return true
    }

    var glyphCount: Int { glyphs.count }
    var rectCount: Int { rects.count }

    /// Encode and present. `size` is the drawable size in points.
    func present(
        to layer: CAMetalLayer,
        size: CGSize,
        scroll: CGPoint,
        background: NSColor
    ) {
        guard size.width > 0, size.height > 0 else { return }
        guard let drawable = layer.nextDrawable() else { return }
        encode(into: drawable.texture, size: size, scroll: scroll, background: background) {
            $0.present(drawable)
        }
    }

    /// Encode this frame into any texture. Split out of `present` so the whole
    /// pipeline — atlas, positioning, blending — can be verified against real
    /// pixels offscreen, without a window or a 12-minute app build.
    func encode(
        into target: MTLTexture,
        size: CGSize,
        scroll: CGPoint,
        background: NSColor,
        beforeCommit: (MTLCommandBuffer) -> Void = { _ in }
    ) {
        guard size.width > 0, size.height > 0 else { return }

        inFlight.wait()
        frameIndex = (frameIndex + 1) % 3
        let rb = rectBuffers[frameIndex]
        let gb = glyphBuffers[frameIndex]

        let rectBytes = rects.count * MemoryLayout<RectInstance>.stride
        let glyphBytes = glyphs.count * MemoryLayout<GlyphInstance>.stride
        // A viewport that overflows the buffer is a bug, not a resize: drop the
        // overflow rather than scribble past the allocation.
        let rectN = min(rects.count, rb.length / MemoryLayout<RectInstance>.stride)
        let glyphN = min(glyphs.count, gb.length / MemoryLayout<GlyphInstance>.stride)
        if rectN > 0 {
            rects.withUnsafeBytes { src in
                rb.contents().copyMemory(
                    from: src.baseAddress!,
                    byteCount: min(rectBytes, rectN * MemoryLayout<RectInstance>.stride)
                )
            }
        }
        if glyphN > 0 {
            glyphs.withUnsafeBytes { src in
                gb.contents().copyMemory(
                    from: src.baseAddress!,
                    byteCount: min(glyphBytes, glyphN * MemoryLayout<GlyphInstance>.stride)
                )
            }
        }

        var uniforms = Uniforms(
            scale: SIMD2(Float(2.0 / size.width), Float(2.0 / size.height)),
            origin: SIMD2(-1, 1),
            scroll: SIMD2(Float(-scroll.x), Float(-scroll.y)),
            _pad: .zero
        )

        let bg = background.usingColorSpace(.sRGB) ?? .black
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = target
        pass.colorAttachments[0].loadAction = .clear
        pass.colorAttachments[0].storeAction = .store
        pass.colorAttachments[0].clearColor = MTLClearColor(
            red: Double(bg.redComponent),
            green: Double(bg.greenComponent),
            blue: Double(bg.blueComponent),
            alpha: 1
        )

        guard let cmd = queue.makeCommandBuffer(),
              let enc = cmd.makeRenderCommandEncoder(descriptor: pass)
        else {
            inFlight.signal()
            return
        }

        if rectN > 0 {
            enc.setRenderPipelineState(rectPipeline)
            enc.setVertexBuffer(rb, offset: 0, index: 0)
            enc.setVertexBytes(&uniforms, length: MemoryLayout<Uniforms>.stride, index: 1)
            enc.drawPrimitives(
                type: .triangleStrip, vertexStart: 0, vertexCount: 4,
                instanceCount: rectN
            )
        }
        if glyphN > 0 {
            enc.setRenderPipelineState(glyphPipeline)
            enc.setVertexBuffer(gb, offset: 0, index: 0)
            enc.setVertexBytes(&uniforms, length: MemoryLayout<Uniforms>.stride, index: 1)
            enc.setFragmentTexture(atlas.texture, index: 0)
            enc.setFragmentSamplerState(sampler, index: 0)
            enc.drawPrimitives(
                type: .triangleStrip, vertexStart: 0, vertexCount: 4,
                instanceCount: glyphN
            )
        }
        enc.endEncoding()

        cmd.addCompletedHandler { [inFlight] _ in inFlight.signal() }
        beforeCommit(cmd)
        cmd.commit()
    }
}
