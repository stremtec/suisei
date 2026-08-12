import AppKit

/// The tab strip, hosted in AppKit.
///
/// See `docs/SUISEI-TAB-STRIP-HOST.md`. The model (`TabStripModel`) and the
/// geometry (`TabStripLayout`) each got a specification and a rewrite and have
/// not produced a defect since. This is the third layer, which got neither:
/// where the viewport width comes from, who owns a press, and how the row moves.
/// Every remaining defect was in it, and each one was an unstated rule.
///
/// The three rules, and how this structure makes each one true rather than
/// maintained:
///
/// **H1 — the viewport is not measured from the SwiftUI layout.** It is derived
/// from `NSWindow.contentLayoutRect`, which does not animate when the sidebar
/// opens. The old strip took its width from a `GeometryReader` inside the
/// animating hierarchy, so the whole run — chips, band, "+", and the × hit rect
/// — was recomputed on every frame of a sidebar toggle. Measured: 1120 → 820 →
/// 1120. Four fixes were aimed at that and all four were misdiagnoses, the last
/// of them moving the row out of the root `ZStack` entirely, which changed
/// nothing. Reading the window is the only way to stop asking a moving thing.
///
/// **H2 — one `originX`, one instant.** `currentFrame()` is the single function
/// that says where everything is, and both `draw(_:)` and `region(at:)` call
/// it. The old strip had a SwiftUI `Layout` interpolating `originX` privately
/// per frame while the AppKit hit test read the settled value out of a captured
/// struct; there was no mechanism by which they could agree, and they did not.
///
/// **H3 — one press owner.** `mouseDown` resolves the point to a `Region` and
/// dispatches. Window dragging is performed here for `.empty`, rather than by a
/// separate layer whose hit-testing is toggled by hover state. The old strip
/// had three participants — the catcher, a `WindowDragGesture` layer and a
/// SwiftUI `Button` — each deciding independently whether a press was theirs,
/// which is why the "+" could not be clicked by any of them.
final class TabStripHostView: NSView {

    // MARK: - Inputs

    /// Everything the strip draws, in order. Set by the representable.
    var tabs: [TabItem] = [] {
        didSet { guard tabs != oldValue else { return }; tabsChanged(from: oldValue) }
    }
    var palette = TabStripPalette.dark { didSet { needsDisplay = true } }
    var actions = TabStripActions()
    /// Tabs the engine has beyond the ABI cap; drawn as a "+N" counter.
    var overflowCount: Int = 0 { didSet { needsDisplay = true } }

    /// Window-space keep-out at each end: traffic lights and the sidebar toggle
    /// on the left, the document toolbar on the right.
    ///
    /// A CONSTANT, deliberately. Feeding the sidebar's live width in here is
    /// what made the run move while the sidebar animated — the thing H1 exists
    /// to stop. It narrows the viewport; it never moves the centre.
    var leadingInset: CGFloat = 150 { didSet { needsDisplay = true } }
    var trailingInset: CGFloat = 150 { didSet { needsDisplay = true } }

    // MARK: - Owned state

    /// Points scrolled from the run's leading edge. Owned here, never observed
    /// back from a scroll view a pass late.
    private var scrollOffset: CGFloat = 0
    /// The origin the run is drawn at RIGHT NOW. `currentFrame()` uses this,
    /// so a press mid-animation resolves against what is on screen.
    private var liveOrigin: CGFloat?
    private var originAnimation: OriginAnimation?
    private var displayLink: CVDisplayLink?

    private var hoveredSlot: Int?
    /// True while the pointer is over the run or the "+", which is what reveals
    /// the "+" — not merely being somewhere in the titlebar row.
    private var pointerInStrip = false
    private var pressedSlot: Int?
    private var heldSlot: Int?
    private var dragStartX: CGFloat = 0
    private var dragMoved = false
    private var lastSwapX: CGFloat?
    /// A trackpad flick is one direct phase plus a long momentum tail; both are
    /// one gesture and may take only one fold step.
    private var foldGestureStepped = false
    private var lastPreciseScrollAt: TimeInterval = 0
    private var lastWheelFoldAt: TimeInterval = 0

    // MARK: - Setup

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layerContentsRedrawPolicy = .onSetNeedsDisplay
        postsFrameChangedNotifications = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { false }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    /// The strip owns every press inside its own regions, including the drag
    /// that moves the window. Leaving this true hands AppKit the press before
    /// any of the routing below runs — that is what the old catcher's comment
    /// called "THE fix", and it is still the fix.
    override var mouseDownCanMoveWindow: Bool { false }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        NotificationCenter.default.removeObserver(self)
        guard let window else { stopDisplayLink(); return }
        for name: NSNotification.Name in [
            NSWindow.didResizeNotification,
            NSWindow.didEndLiveResizeNotification,
        ] {
            NotificationCenter.default.addObserver(
                self, selector: #selector(windowGeometryChanged),
                name: name, object: window
            )
        }
        needsDisplay = true
    }

    @objc private func windowGeometryChanged() {
        // A resize re-centres the run. It does NOT animate: the window is
        // already moving under the pointer and a second easing on top reads as
        // lag. Only scrolling and tab-set changes animate.
        liveOrigin = nil
        originAnimation = nil
        needsDisplay = true
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
        if let link = displayLink { CVDisplayLinkStop(link) }
    }

    // MARK: - Geometry
    //
    // ONE function answers "where is everything". Both the drawing and the hit
    // test call it, so they cannot describe different strips.

    /// Where the strip is, for one instant.
    struct Frame {
        let layout: TabStripLayout
        /// Leading edge of the VIEWPORT in this view's coordinates. Chip x's
        /// are `viewportX + layout.originX + chip.x`.
        let viewportX: CGFloat
        /// The origin actually in use — the animator's value while a move is in
        /// flight, the layout's own otherwise.
        let originX: CGFloat
        /// Vertical origin of the 24pt chip row within the view.
        let rowY: CGFloat

        func chipRect(_ chip: TabStripLayout.Chip) -> CGRect {
            CGRect(
                x: viewportX + originX + chip.x, y: rowY,
                width: chip.width, height: TabChipBox.height
            )
        }

        /// The close glyph's box — the DRAWN one. The hit rect is this grown by
        /// `TabChipBox.closeHitInset`, which `TabStripLayout.closeRect` does.
        func closeRect(_ chip: TabStripLayout.Chip) -> CGRect {
            let slot = TabChipBox.trailingSlotWidth
            return CGRect(
                x: viewportX + originX + chip.maxX - TabChipBox.closeSlotInset,
                y: rowY + (TabChipBox.height - slot) / 2,
                width: slot, height: slot
            )
        }

        var plusRect: CGRect {
            CGRect(
                x: viewportX + layout.plusX, y: rowY,
                width: TabStripLayout.plusWidth, height: TabChipBox.height
            )
        }

        /// The whole run plus its "+", which is the region that counts as
        /// "in the strip" for revealing the button and for declining a
        /// window drag.
        var activeRect: CGRect {
            let runLeft = viewportX + originX
            let runRight = max(runLeft + layout.contentWidth, plusRect.maxX)
            return CGRect(
                x: runLeft, y: rowY,
                width: max(0, runRight - runLeft), height: TabChipBox.height
            )
        }
    }

    /// The viewport, in WINDOW coordinates: the widest span that is symmetric
    /// about the window's centre and clears both insets.
    ///
    /// Symmetry is what makes this "창 기준 중앙" rather than "에디터 기준
    /// 중앙" — `TabStripLayout` centres the run in its viewport, so a viewport
    /// centred on the window puts the run on the window's centre line for free,
    /// and an asymmetric pair of insets only makes the viewport narrower. The
    /// centre does not move when one side's inset changes, which is the whole
    /// complaint about the previous strip.
    private func viewportInWindow() -> (x: CGFloat, width: CGFloat)? {
        guard let window else { return nil }
        let content = window.contentLayoutRect
        guard content.width > 0 else { return nil }
        let centre = content.midX
        let half = min(
            centre - (content.minX + leadingInset),
            (content.maxX - trailingInset) - centre
        )
        guard half > 1 else { return nil }
        return (x: centre - half, width: half * 2)
    }

    /// Where everything is, right now.
    func currentFrame() -> Frame? {
        guard let vp = viewportInWindow() else { return nil }
        let layout = TabStripLayout(
            tabs: tabs.map { (stableId: $0.stableId, group: $0.group) },
            viewportWidth: vp.width,
            scrollOffset: scrollOffset,
            widthFor: { [tabs] slot in
                let t = tabs[slot]
                return TabChipMetrics.width(
                    title: t.title, active: t.active,
                    isLayout: t.isLayout, deleted: t.deleted
                )
            }
        )
        // Window x → this view's x. Only x matters; flipping affects y alone.
        let selfOriginInWindow = convert(NSPoint.zero, to: nil).x
        return Frame(
            layout: layout,
            viewportX: vp.x - selfOriginInWindow,
            originX: liveOrigin ?? layout.originX,
            rowY: ((bounds.height - TabChipBox.height) / 2).rounded()
        )
    }

    /// What is under a point, in this view's coordinates.
    enum Region: Equatable {
        case close(slot: Int)
        case chip(slot: Int)
        case plus
        case empty
    }

    func region(at point: CGPoint) -> Region {
        guard let f = currentFrame() else { return .empty }
        // Close before chip: the × sits inside its chip, and it wins there.
        for chip in f.layout.chips {
            let hit = f.closeRect(chip)
                .insetBy(dx: -TabChipBox.closeHitInset, dy: -TabChipBox.closeHitInset)
            if hit.contains(point) { return .close(slot: chip.slot) }
        }
        for chip in f.layout.chips where f.chipRect(chip).contains(point) {
            return .chip(slot: chip.slot)
        }
        if f.plusRect.insetBy(dx: -2, dy: -2).contains(point) { return .plus }
        return .empty
    }

    // MARK: - Hit testing
    //
    // The strip spans the whole titlebar row so its geometry cannot be moved by
    // SwiftUI, but it must not swallow what lives in the reserved ends — the
    // traffic lights, the sidebar toggle, the document toolbar.

    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let local = superview.map({ convert($0.convert(point, to: self), from: self) })
            ?? nil as NSPoint?
        else { return nil }
        return hitTestLocal(local)
    }

    private func hitTestLocal(_ local: NSPoint) -> NSView? {
        guard let vp = viewportInWindow() else { return nil }
        let selfOriginInWindow = convert(NSPoint.zero, to: nil).x
        let vpX = vp.x - selfOriginInWindow
        guard local.x >= vpX, local.x <= vpX + vp.width else { return nil }
        guard bounds.contains(local) else { return nil }
        return self
    }

    // MARK: - Tracking

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: bounds,
            options: [
                .activeInKeyWindow, .mouseMoved, .mouseEnteredAndExited,
                .inVisibleRect,
            ],
            owner: self
        ))
    }

    override func mouseMoved(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        updateHover(at: p)
        if heldSlot != nil { advanceDrag(to: p.x) }
    }

    override func mouseExited(with event: NSEvent) { clearHover() }

    private func updateHover(at p: CGPoint) {
        let inStrip = currentFrame()?.activeRect
            .insetBy(dx: -6, dy: -4)
            .contains(p) ?? false
        // The × is inside a chip, so hovering it still hovers the chip. That is
        // what keeps the glyph on screen while the pointer travels to it — the
        // old strip resolved hover and the × separately and the glyph vanished
        // as the pointer arrived.
        let slot: Int? = switch region(at: p) {
        case .chip(let s), .close(let s): s
        case .plus, .empty: nil
        }
        guard slot != hoveredSlot || inStrip != pointerInStrip else { return }
        hoveredSlot = slot
        pointerInStrip = inStrip
        needsDisplay = true
    }

    private func clearHover() {
        guard hoveredSlot != nil || pointerInStrip else { return }
        hoveredSlot = nil
        pointerInStrip = false
        needsDisplay = true
    }

    // MARK: - Press routing (H3)

    override func mouseDown(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        dragStartX = p.x
        dragMoved = false
        lastSwapX = nil
        heldSlot = nil

        switch region(at: p) {
        case .empty:
            // Titlebar behaviour, performed here rather than by a separate
            // gesture layer that has to be switched off over the tabs.
            if event.clickCount >= 2 {
                window?.performZoom(nil)
            } else {
                window?.performDrag(with: event)
            }
        case .plus:
            pressedSlot = nil
            actions.plusMenu?(self, convert(p, to: nil), event)
        case .close(let slot):
            pressedSlot = slot
            needsDisplay = true
        case .chip(let slot):
            heldSlot = slot
            pressedSlot = slot
            needsDisplay = true
        }
    }

    override func mouseDragged(with event: NSEvent) {
        advanceDrag(to: convert(event.locationInWindow, from: nil).x)
    }

    override func mouseUp(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        if !dragMoved {
            switch region(at: p) {
            case .close(let slot):
                if pressedSlot == slot { actions.close?(slot) }
            case .chip(let slot):
                if event.clickCount >= 2 {
                    actions.doubleClick?(slot)
                } else {
                    actions.click?(slot)
                }
            case .plus, .empty:
                break
            }
        }
        heldSlot = nil
        pressedSlot = nil
        dragMoved = false
        lastSwapX = nil
        actions.dragEnded?()
        needsDisplay = true
    }

    override func rightMouseDown(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        guard case .chip(let slot) = region(at: p) else {
            super.rightMouseDown(with: event)
            return
        }
        actions.contextMenu?(self, slot, event)
    }

    /// Reorder, one neighbour at a time, decided by the neighbour's MIDPOINT.
    /// Bounds would swap on a touch, and the swap moves chips back under the
    /// cursor, which oscillates.
    private func advanceDrag(to x: CGFloat) {
        guard let from = heldSlot else { return }
        if !dragMoved, abs(x - dragStartX) > 3 {
            dragMoved = true
            actions.dragBegan?(from)
        }
        guard dragMoved else { return }
        if let last = lastSwapX, abs(x - last) < 6 { return }
        guard let f = currentFrame() else { return }
        let stripX = x - f.viewportX
        guard let to = f.layout.dragTarget(held: from, x: stripX), to != from
        else { return }
        actions.reorder?(from, to)
        heldSlot = to
        lastSwapX = x
        needsDisplay = true
    }

    // MARK: - Scrolling and folding

    override func scrollWheel(with event: NSEvent) {
        observeScrollGesture(event)
        guard isFoldFlick(event) else {
            // A trackpad's horizontal component is `scrollingDeltaX`. A plain
            // wheel has none, and a tab strip is expected to scroll from one
            // (every browser does), so a vertical wheel that is not a fold
            // flick drives the run sideways — but only a WHEEL. Letting a
            // trackpad's vertical delta do it made an ordinary two-finger
            // scroll over the strip shove the tabs sideways.
            let raw: CGFloat = if event.scrollingDeltaX != 0 {
                event.scrollingDeltaX
            } else if !event.hasPreciseScrollingDeltas {
                event.scrollingDeltaY * 24
            } else {
                0
            }
            if raw != 0, scrollRun(by: raw) { return }
            super.scrollWheel(with: event)
            return
        }
        if event.hasPreciseScrollingDeltas {
            guard !foldGestureStepped else { return }
            foldGestureStepped = true
        } else {
            guard event.timestamp - lastWheelFoldAt >= 0.6 else { return }
            lastWheelFoldAt = event.timestamp
        }
        if isUpward(event) { actions.foldUp?() } else { actions.foldDown?() }
    }

    /// Gesture boundaries, so a momentum tail never takes a second fold step.
    private func observeScrollGesture(_ event: NSEvent) {
        guard event.hasPreciseScrollingDeltas else { return }
        if event.phase.contains(.began)
            || event.timestamp - lastPreciseScrollAt > 0.25
        {
            foldGestureStepped = false
        }
        lastPreciseScrollAt = event.timestamp
    }

    /// A fast, dominantly-vertical flick. Both halves matter: without the
    /// dominant-axis test a long sideways scroll folds the layout by accident,
    /// and without the velocity floor so does a slow drift.
    private func isFoldFlick(_ e: NSEvent) -> Bool {
        let dy = e.scrollingDeltaY, dx = e.scrollingDeltaX
        guard abs(dy) > abs(dx) * 1.5 else { return false }
        // The floor's UNIT is per-device: a trackpad reports points and drifts,
        // a wheel reports detents and one detent is already deliberate.
        return e.hasPreciseScrollingDeltas ? abs(dy) >= 6 : abs(dy) >= 1
    }

    /// `scrollingDeltaY` is in content space, so its sign flips with "natural
    /// scrolling" — read raw, the gesture means the opposite on half the
    /// machines.
    private func isUpward(_ e: NSEvent) -> Bool {
        e.isDirectionInvertedFromDevice
            ? e.scrollingDeltaY < 0 : e.scrollingDeltaY > 0
    }

    @discardableResult
    private func scrollRun(by delta: CGFloat) -> Bool {
        guard let f = currentFrame(), f.layout.overflow else { return false }
        let maxScroll = max(0, f.layout.contentWidth - f.layout.viewportWidth)
        let next = min(max(0, scrollOffset - delta), maxScroll)
        guard next != scrollOffset else { return false }
        scrollOffset = next
        // A scroll is direct manipulation: it tracks the finger, it does not
        // ease behind it.
        liveOrigin = nil
        originAnimation = nil
        needsDisplay = true
        return true
    }

    /// Bring a slot fully into view if it is not already, easing there.
    func reveal(slot: Int) {
        guard let f = currentFrame(),
              let next = f.layout.scrollToReveal(
                  slot: slot, currentOffset: scrollOffset
              ),
              next != scrollOffset
        else { return }
        let before = f.originX
        scrollOffset = next
        guard let after = currentFrame()?.layout.originX else { return }
        animateOrigin(from: before, to: after, duration: 0.20)
    }

    // MARK: - Motion (H2)
    //
    // The animated origin is a STORED property that `currentFrame()` returns,
    // so the hit test resolves against the row's live position. The old strip
    // interpolated it inside a SwiftUI `Layout`, where only the placement could
    // see it.

    private struct OriginAnimation {
        let from: CGFloat
        let to: CGFloat
        let start: TimeInterval
        let duration: TimeInterval
    }

    private func tabsChanged(from old: [TabItem]) {
        // A changed tab set moves the run's centre. Ease from wherever it is
        // drawn to wherever it now belongs.
        let before = liveOrigin
        needsDisplay = true
        guard let to = currentFrame()?.layout.originX else { return }
        guard let from = before ?? previousOrigin(for: old), from != to else { return }
        animateOrigin(from: from, to: to, duration: 0.22)
    }

    /// The origin the previous tab set was resting at, for the ease's start.
    private func previousOrigin(for old: [TabItem]) -> CGFloat? {
        guard let vp = viewportInWindow() else { return nil }
        return TabStripLayout(
            tabs: old.map { (stableId: $0.stableId, group: $0.group) },
            viewportWidth: vp.width,
            scrollOffset: scrollOffset,
            widthFor: { slot in
                let t = old[slot]
                return TabChipMetrics.width(
                    title: t.title, active: t.active,
                    isLayout: t.isLayout, deleted: t.deleted
                )
            }
        ).originX
    }

    private func animateOrigin(
        from: CGFloat, to: CGFloat, duration: TimeInterval
    ) {
        guard from != to else { return }
        liveOrigin = from
        originAnimation = OriginAnimation(
            from: from, to: to,
            start: CACurrentMediaTime(), duration: duration
        )
        startDisplayLink()
    }

    private func startDisplayLink() {
        if displayLink == nil {
            var link: CVDisplayLink?
            CVDisplayLinkCreateWithActiveCGDisplays(&link)
            guard let link else { return }
            CVDisplayLinkSetOutputHandler(link) { [weak self] _, _, _, _, _ in
                DispatchQueue.main.async { self?.stepOrigin() }
                return kCVReturnSuccess
            }
            displayLink = link
        }
        if let link = displayLink, !CVDisplayLinkIsRunning(link) {
            CVDisplayLinkStart(link)
        }
    }

    private func stopDisplayLink() {
        guard let link = displayLink, CVDisplayLinkIsRunning(link) else { return }
        CVDisplayLinkStop(link)
    }

    private func stepOrigin() {
        guard let a = originAnimation else { stopDisplayLink(); return }
        let t = min(1, (CACurrentMediaTime() - a.start) / a.duration)
        // Ease-out cubic: leaves quickly, settles softly. Matches the
        // `.snappy`-family feel the strip's other motion uses without needing a
        // spring solver here.
        let eased = 1 - pow(1 - t, 3)
        liveOrigin = a.from + (a.to - a.from) * CGFloat(eased)
        if t >= 1 {
            originAnimation = nil
            liveOrigin = nil
            stopDisplayLink()
        }
        needsDisplay = true
    }

    // MARK: - Drawing
    //
    // Every rect drawn here comes from the same `Frame` the hit test reads.
    // There is no second set of coordinates to keep equal.

    override func draw(_ dirtyRect: NSRect) {
        guard let f = currentFrame(), let ctx = NSGraphicsContext.current?.cgContext
        else { return }

        let viewport = CGRect(
            x: f.viewportX, y: 0, width: f.layout.viewportWidth, height: bounds.height
        )
        // Chips dissolve into the clip at both ends. Drawn into a transparency
        // layer so the gradient masks the strip's own content and nothing else.
        ctx.saveGState()
        ctx.clip(to: viewport)
        ctx.beginTransparencyLayer(auxiliaryInfo: nil)

        drawBands(f, in: ctx)
        drawActiveCapsule(f, in: ctx)
        drawHoverCapsule(f, in: ctx)
        for chip in f.layout.chips { drawChip(chip, f, in: ctx) }
        drawPlus(f, in: ctx)
        drawOverflowCounter(f, in: ctx)

        if f.layout.overflow { applyEdgeFade(viewport, in: ctx) }
        ctx.endTransparencyLayer()
        ctx.restoreGState()
    }

    private func drawBands(_ f: Frame, in ctx: CGContext) {
        var seen = Set<UInt64>()
        for chip in f.layout.chips where chip.group != 0 {
            guard seen.insert(chip.group).inserted,
                  let extent = f.layout.bandExtent(group: chip.group)
            else { continue }
            let rect = CGRect(
                x: f.viewportX + extent.minX - f.layout.originX + f.originX,
                y: f.rowY,
                width: max(0, extent.maxX - extent.minX),
                height: TabChipBox.height
            )
            let path = CGPath(
                roundedRect: rect.insetBy(dx: 0.5, dy: 0.5),
                cornerWidth: 12, cornerHeight: 12, transform: nil
            )
            ctx.addPath(path)
            ctx.setFillColor(palette.groupFill.cgColor)
            ctx.fillPath()
            ctx.addPath(path)
            ctx.setStrokeColor(palette.groupStroke.cgColor)
            ctx.setLineWidth(1)
            ctx.strokePath()
        }
    }

    private func drawActiveCapsule(_ f: Frame, in ctx: CGContext) {
        guard let chip = f.layout.chips.first(where: { c in
            tabs.first { $0.id == c.slot }?.active == true
        }) else { return }
        fillCapsule(f.chipRect(chip), palette.activeFill, in: ctx)
    }

    private func drawHoverCapsule(_ f: Frame, in ctx: CGContext) {
        guard let slot = hoveredSlot,
              let chip = f.layout.chips.first(where: { $0.slot == slot }),
              tabs.first(where: { $0.id == slot })?.active != true
        else { return }
        let fill = chip.group != 0 ? palette.hoverFillInGroup : palette.hoverFill
        fillCapsule(f.chipRect(chip), fill, in: ctx)
    }

    private func fillCapsule(_ rect: CGRect, _ color: NSColor, in ctx: CGContext) {
        ctx.addPath(CGPath(
            roundedRect: rect,
            cornerWidth: rect.height / 2, cornerHeight: rect.height / 2,
            transform: nil
        ))
        ctx.setFillColor(color.cgColor)
        ctx.fillPath()
    }

    private func drawChip(
        _ chip: TabStripLayout.Chip, _ f: Frame, in ctx: CGContext
    ) {
        guard let tab = tabs.first(where: { $0.id == chip.slot }) else { return }
        let rect = f.chipRect(chip)
        guard rect.maxX > f.viewportX - 40,
              rect.minX < f.viewportX + f.layout.viewportWidth + 40
        else { return }   // fully clipped; skip the text shaping

        let inGroup = tab.isLayout || tab.group != 0
        let ink = palette.ink(active: tab.active, inGroup: inGroup)
        let iconColor = tab.deleted
            ? NSColor.systemOrange
            : palette.iconInk(active: tab.active, inGroup: inGroup)

        ctx.saveGState()
        if pressedSlot == chip.slot {
            // Press feedback, about the chip's own centre.
            ctx.translateBy(x: rect.midX, y: rect.midY)
            ctx.scaleBy(x: 0.96, y: 0.96)
            ctx.translateBy(x: -rect.midX, y: -rect.midY)
        }

        var pen = rect.minX + TabChipBox.horizontalPadding
        let iconName = TabChipMetrics.symbolName(
            isLayout: tab.isLayout, deleted: tab.deleted
        )
        let iconW = TabChipMetrics.symbolWidth(iconName)
        drawSymbol(
            iconName, color: iconColor,
            in: CGRect(
                x: pen, y: rect.midY - TabChipBox.height / 2,
                width: iconW, height: TabChipBox.height
            )
        )
        pen += iconW + TabChipBox.interItemGap

        let titleW = rect.maxX - TabChipBox.closeSlotInset
            - TabChipBox.interItemGap - pen
        if titleW > 0 {
            drawTitle(
                tab, color: ink,
                in: CGRect(x: pen, y: rect.minY, width: titleW, height: rect.height)
            )
        }

        drawTrailingSlot(tab, chip, f, in: ctx)
        ctx.restoreGState()
    }

    /// The dirty dot, or the × once the chip is hovered. One slot, never both.
    private func drawTrailingSlot(
        _ tab: TabItem, _ chip: TabStripLayout.Chip, _ f: Frame, in ctx: CGContext
    ) {
        let slot = f.closeRect(chip)
        let showClose = hoveredSlot == chip.slot
        if showClose {
            ctx.addEllipse(in: slot)
            ctx.setFillColor(palette.closeWell.cgColor)
            ctx.fillPath()
            let inGroup = tab.isLayout || tab.group != 0
            drawSymbol(
                "xmark",
                color: palette.ink(active: false, inGroup: inGroup)
                    .withAlphaComponent(0.90),
                in: slot, pointSize: 8, weight: .bold
            )
        } else if tab.dirty {
            let d: CGFloat = 6
            ctx.addEllipse(in: CGRect(
                x: slot.midX - d / 2, y: slot.midY - d / 2, width: d, height: d
            ))
            ctx.setFillColor(NSColor.systemOrange.cgColor)
            ctx.fillPath()
        }
    }

    private func drawPlus(_ f: Frame, in ctx: CGContext) {
        guard pointerInStrip else { return }
        let rect = f.plusRect
        // Literal "+", not an SF Symbol: rendered through the same text layout
        // as the tab labels it sits beside, so it lands on their optical line
        // instead of a symbol's alignment-rect centre (the recurring "+가 위로
        // 튐"). `plusInkNudge` is the last sub-point of that.
        let s = NSAttributedString(string: "+", attributes: [
            .font: NSFont.systemFont(ofSize: 20, weight: .regular),
            .foregroundColor: palette.dim,
        ])
        let size = s.size()
        s.draw(at: CGPoint(
            x: (rect.midX - size.width / 2).rounded(),
            y: (rect.midY - size.height / 2 + TabStripHostView.plusInkNudge).rounded()
        ))
    }

    /// Lands the "+" on the tab labels' optical line. Frame height cancels out
    /// of the centring arithmetic, so this is purely the glyph-ink difference:
    /// a "+" is drawn on the maths axis, whose centre sits above a mixed-case
    /// line's.
    private static let plusInkNudge: CGFloat = -1.0

    private func drawOverflowCounter(_ f: Frame, in ctx: CGContext) {
        guard overflowCount > 0 else { return }
        let s = NSAttributedString(string: "+\(overflowCount)", attributes: [
            .font: NSFont.systemFont(ofSize: 11, weight: .semibold),
            .foregroundColor: palette.dim,
        ])
        let size = s.size()
        s.draw(at: CGPoint(
            x: f.plusRect.maxX,
            y: (f.rowY + (TabChipBox.height - size.height) / 2).rounded()
        ))
    }

    private func applyEdgeFade(_ viewport: CGRect, in ctx: CGContext) {
        let w = TabStripHostView.edgeFadeWidth
        ctx.setBlendMode(.destinationIn)
        let space = CGColorSpaceCreateDeviceGray()
        guard let gradient = CGGradient(
            colorsSpace: space,
            colors: [
                NSColor(white: 0, alpha: 0).cgColor,
                NSColor(white: 0, alpha: 1).cgColor,
            ] as CFArray,
            locations: [0, 1]
        ) else { return }

        ctx.saveGState()
        ctx.clip(to: CGRect(
            x: viewport.minX, y: viewport.minY, width: w, height: viewport.height
        ))
        ctx.drawLinearGradient(
            gradient,
            start: CGPoint(x: viewport.minX, y: 0),
            end: CGPoint(x: viewport.minX + w, y: 0),
            options: []
        )
        ctx.restoreGState()

        ctx.saveGState()
        ctx.clip(to: CGRect(
            x: viewport.maxX - w, y: viewport.minY, width: w, height: viewport.height
        ))
        ctx.drawLinearGradient(
            gradient,
            start: CGPoint(x: viewport.maxX, y: 0),
            end: CGPoint(x: viewport.maxX - w, y: 0),
            options: []
        )
        ctx.restoreGState()
        ctx.setBlendMode(.normal)
    }

    static let edgeFadeWidth: CGFloat = 14

    // MARK: - Text and symbols

    private func drawTitle(_ tab: TabItem, color: NSColor, in rect: CGRect) {
        let para = NSMutableParagraphStyle()
        para.lineBreakMode = .byTruncatingTail
        var attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(
                ofSize: 12, weight: tab.active ? .semibold : .regular
            ),
            .foregroundColor: color,
            .paragraphStyle: para,
        ]
        if tab.deleted {
            attrs[.strikethroughStyle] = NSUnderlineStyle.single.rawValue
            attrs[.strikethroughColor] = NSColor.systemOrange
        }
        let s = NSAttributedString(string: tab.title, attributes: attrs)
        let h = s.size().height
        s.draw(with: CGRect(
            x: rect.minX, y: (rect.midY - h / 2).rounded(),
            width: rect.width, height: h
        ), options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
    }

    private func drawSymbol(
        _ name: String, color: NSColor, in rect: CGRect,
        pointSize: CGFloat = 10, weight: NSFont.Weight = .regular
    ) {
        let cfg = NSImage.SymbolConfiguration(pointSize: pointSize, weight: weight)
            .applying(NSImage.SymbolConfiguration(paletteColors: [color]))
        guard let image = NSImage(systemSymbolName: name, accessibilityDescription: nil)?
            .withSymbolConfiguration(cfg)
        else { return }
        let size = image.size
        image.draw(in: CGRect(
            x: (rect.midX - size.width / 2).rounded(),
            y: (rect.midY - size.height / 2).rounded(),
            width: size.width, height: size.height
        ))
    }
}

// MARK: - Inputs

/// Colours the strip draws with, resolved from the editor theme by the caller
/// so this view never reaches for `Color.accentColor` (which is the SYSTEM
/// accent and ignores `.tint`).
struct TabStripPalette: Equatable {
    var isLight: Bool
    var accent: NSColor
    var fg: NSColor
    var dim: NSColor
    var groupFill: NSColor
    var groupStroke: NSColor
    var activeFill: NSColor
    var hoverFill: NSColor
    var hoverFillInGroup: NSColor
    var closeWell: NSColor

    func ink(active: Bool, inGroup: Bool) -> NSColor {
        if inGroup { return NSColor.black.withAlphaComponent(active ? 1 : 0.72) }
        return active ? fg : dim
    }

    func iconInk(active: Bool, inGroup: Bool) -> NSColor {
        if inGroup { return NSColor.black.withAlphaComponent(active ? 1 : 0.72) }
        return active ? accent : dim.withAlphaComponent(0.85)
    }

    static let dark = TabStripPalette(
        isLight: false,
        accent: .controlAccentColor,
        fg: .labelColor,
        dim: .secondaryLabelColor,
        groupFill: NSColor.systemBlue.withAlphaComponent(0.32),
        groupStroke: NSColor.systemBlue.withAlphaComponent(0.55),
        activeFill: NSColor.labelColor.withAlphaComponent(0.14),
        hoverFill: NSColor.labelColor.withAlphaComponent(0.10),
        hoverFillInGroup: NSColor.black.withAlphaComponent(0.08),
        closeWell: NSColor.labelColor.withAlphaComponent(0.16)
    )
}

/// What the strip does when something is pressed. Every one of these is
/// dispatched from `mouseDown`/`mouseUp`, so there is one place that decides.
struct TabStripActions {
    var click: ((Int) -> Void)?
    var doubleClick: ((Int) -> Void)?
    var close: ((Int) -> Void)?
    var reorder: ((Int, Int) -> Void)?
    var dragBegan: ((Int) -> Void)?
    var dragEnded: (() -> Void)?
    var foldUp: (() -> Void)?
    var foldDown: (() -> Void)?
    var plusMenu: ((NSView, CGPoint, NSEvent) -> Void)?
    var contextMenu: ((NSView, Int, NSEvent) -> Void)?
}
