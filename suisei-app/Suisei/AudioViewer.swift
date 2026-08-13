//  AudioViewer.swift
//  An audio file in a pane, shaped like Apple Music's album page.
//
//  What is taken from Music: the large square artwork, the title with the
//  artist under it IN THE ACCENT COLOUR, the small dim `album · year` line
//  beneath that, capsule transport buttons, and a floating rounded card at the
//  bottom rather than a bar welded to the edge.
//
//  What is not: Music is a library browser and this is one file. The track
//  list becomes a waveform — the thing you actually want from a file viewer,
//  because it shows where the sound IS — and the footer states what the file
//  is, which a music player never has to tell you.
//
//  Playback is `AVPlayer`, which handles every format the system can decode.
//  The waveform comes from `AVAudioFile` read in chunks off the main thread.
//  Now Playing and the media keys are wired, but only while sound is actually
//  coming out — see `AudioPlayerModel.becomeNowPlaying`.

import AVFoundation
import AppKit
import MediaPlayer
import SwiftUI

// MARK: - Model

@MainActor
final class AudioPlayerModel: ObservableObject {
    @Published var title = ""
    @Published var artist = ""
    @Published var album = ""
    @Published var year = ""
    @Published var artwork: NSImage?
    @Published var duration: Double = 0
    @Published var elapsed: Double = 0
    @Published var playing = false
    /// Peak amplitude per horizontal bucket, 0…1. Empty until the scan lands,
    /// and empty forever for a file `AVAudioFile` cannot open — the scrubber
    /// falls back to a plain bar rather than the view failing.
    @Published var peaks: [Float] = []
    @Published var format = ""

    private var player: AVPlayer?
    private var timeObserver: Any?
    private var scanTask: Task<Void, Never>?
    private var loadTask: Task<Void, Never>?
    private var url: URL?
    /// Set while this model owns the system's Now Playing slot, so it is only
    /// given back by whoever took it.
    private var isNowPlaying = false

    /// How many buckets the waveform is reduced to. Fixed rather than derived
    /// from the pane width: the scan is the expensive part, panes get resized,
    /// and 1,200 is finer than any pane is wide, so the drawing can average
    /// down to whatever it needs without ever rescanning.
    private static let bucketCount = 1_200

    // MARK: Lifecycle

    func open(_ path: String) {
        guard !path.isEmpty else { return }
        let url = URL(fileURLWithPath: path)
        guard url != self.url else { return }
        close()
        self.url = url

        let item = AVPlayerItem(url: url)
        let player = AVPlayer(playerItem: item)
        self.player = player
        // 20 Hz. The label only needs a few, but this number also drives the
        // playhead across the waveform, and at 10 Hz that visibly steps.
        timeObserver = player.addPeriodicTimeObserver(
            forInterval: CMTime(seconds: 0.05, preferredTimescale: 600),
            queue: .main
        ) { [weak self] t in
            MainActor.assumeIsolated { self?.elapsed = t.seconds }
        }
        // Fall back to the filename immediately, so the pane is never blank
        // while the metadata loads. Real titles overwrite it if they exist.
        title = url.deletingPathExtension().lastPathComponent

        loadTask = Task { [weak self] in await self?.loadMetadata(url) }
        scanTask = Task.detached(priority: .utility) { [weak self] in
            let peaks = Self.scanPeaks(url, buckets: Self.bucketCount)
            await MainActor.run { self?.peaks = peaks }
        }
    }

    func close() {
        resignNowPlaying()
        scanTask?.cancel()
        loadTask?.cancel()
        scanTask = nil
        loadTask = nil
        if let t = timeObserver { player?.removeTimeObserver(t) }
        timeObserver = nil
        player?.pause()
        player = nil
        url = nil
        playing = false
        elapsed = 0
        duration = 0
        peaks = []
        artwork = nil
        title = ""; artist = ""; album = ""; year = ""; format = ""
    }

    deinit {
        // `close()` is main-actor isolated and deinit is not. Everything that
        // outlives this object without it is the Now Playing slot, so hand
        // that back directly.
        if isNowPlaying {
            MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
            MPNowPlayingInfoCenter.default().playbackState = .stopped
        }
    }

    // MARK: Transport

    func toggle() { playing ? pause() : play() }

    func play() {
        guard let player else { return }
        // Restart from the top rather than sitting at the end doing nothing.
        if duration > 0, elapsed >= duration - 0.05 { seek(to: 0) }
        player.play()
        playing = true
        becomeNowPlaying()
    }

    func pause() {
        player?.pause()
        playing = false
        // Keep the Now Playing entry but tell the system it is paused, which
        // is what leaves the Control Center tile showing this track with a
        // play button instead of blanking it.
        MPNowPlayingInfoCenter.default().playbackState = .paused
        pushNowPlayingTime()
    }

    func seek(to seconds: Double) {
        guard let player, duration > 0 else { return }
        let clamped = min(max(0, seconds), duration)
        player.seek(
            to: CMTime(seconds: clamped, preferredTimescale: 600),
            toleranceBefore: .zero, toleranceAfter: .zero
        )
        elapsed = clamped
        pushNowPlayingTime()
    }

    func skip(_ delta: Double) { seek(to: elapsed + delta) }

    // MARK: Now Playing

    /// Claim the system Now Playing slot — but only from `play()`.
    ///
    /// Registering the remote commands at load time would make a text editor
    /// with an audio tab open the target of the keyboard's play button, taking
    /// it from whatever the user was actually listening to. Nothing is claimed
    /// until this pane makes a sound, and `resignNowPlaying` gives it back.
    private func becomeNowPlaying() {
        if !isNowPlaying {
            isNowPlaying = true
            // `Task { @MainActor }` rather than `assumeIsolated`: the remote
            // command centre does not promise which thread it calls a handler
            // on, and `assumeIsolated` does not merely fail off the main
            // thread — it traps. A media key would take the app down.
            let c = MPRemoteCommandCenter.shared()
            c.playCommand.addTarget { [weak self] _ in
                Task { @MainActor in self?.play() }
                return .success
            }
            c.pauseCommand.addTarget { [weak self] _ in
                Task { @MainActor in self?.pause() }
                return .success
            }
            c.togglePlayPauseCommand.addTarget { [weak self] _ in
                Task { @MainActor in self?.toggle() }
                return .success
            }
            c.changePlaybackPositionCommand.addTarget { [weak self] e in
                guard let e = e as? MPChangePlaybackPositionCommandEvent else {
                    return .commandFailed
                }
                let at = e.positionTime
                Task { @MainActor in self?.seek(to: at) }
                return .success
            }
        }
        var info: [String: Any] = [
            MPMediaItemPropertyTitle: title,
            MPMediaItemPropertyArtist: artist,
            MPMediaItemPropertyAlbumTitle: album,
            MPMediaItemPropertyPlaybackDuration: duration,
            MPNowPlayingInfoPropertyElapsedPlaybackTime: elapsed,
            MPNowPlayingInfoPropertyPlaybackRate: 1.0,
        ]
        if let artwork {
            info[MPMediaItemPropertyArtwork] = MPMediaItemArtwork(boundsSize: artwork.size) { _ in
                artwork
            }
        }
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
        MPNowPlayingInfoCenter.default().playbackState = .playing
    }

    private func pushNowPlayingTime() {
        guard isNowPlaying else { return }
        var info = MPNowPlayingInfoCenter.default().nowPlayingInfo ?? [:]
        info[MPNowPlayingInfoPropertyElapsedPlaybackTime] = elapsed
        info[MPNowPlayingInfoPropertyPlaybackRate] = playing ? 1.0 : 0.0
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }

    private func resignNowPlaying() {
        guard isNowPlaying else { return }
        isNowPlaying = false
        let c = MPRemoteCommandCenter.shared()
        c.playCommand.removeTarget(nil)
        c.pauseCommand.removeTarget(nil)
        c.togglePlayPauseCommand.removeTarget(nil)
        c.changePlaybackPositionCommand.removeTarget(nil)
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
        MPNowPlayingInfoCenter.default().playbackState = .stopped
    }

    // MARK: Loading

    private func loadMetadata(_ url: URL) async {
        let asset = AVURLAsset(url: url)
        if let d = try? await asset.load(.duration), d.isNumeric {
            duration = d.seconds
        }
        if let items = try? await asset.load(.commonMetadata) {
            for item in items {
                switch item.commonKey {
                case .commonKeyTitle:
                    if let s = try? await item.load(.stringValue), !s.isEmpty { title = s }
                case .commonKeyArtist, .commonKeyCreator:
                    if artist.isEmpty, let s = try? await item.load(.stringValue) { artist = s }
                case .commonKeyAlbumName:
                    if let s = try? await item.load(.stringValue) { album = s }
                case .commonKeyCreationDate:
                    if let s = try? await item.load(.stringValue) { year = String(s.prefix(4)) }
                case .commonKeyArtwork:
                    if let d = try? await item.load(.dataValue) { artwork = NSImage(data: d) }
                default:
                    break
                }
            }
        }
        format = await Self.describeFormat(asset, url: url)
    }

    /// `MP3 · 320 kbps · 44.1 kHz · Stereo · 4.2 MB` — the part Music never
    /// shows, because Music is playing a song and this is holding a file.
    private static func describeFormat(_ asset: AVURLAsset, url: URL) async -> String {
        var parts: [String] = []
        let ext = url.pathExtension.uppercased()
        if !ext.isEmpty { parts.append(ext) }
        if let track = try? await asset.loadTracks(withMediaType: .audio).first {
            if let rate = try? await track.load(.estimatedDataRate), rate > 0 {
                parts.append("\(Int((rate / 1000).rounded())) kbps")
            }
            if let descs = try? await track.load(.formatDescriptions),
               let d = descs.first,
               let basic = CMAudioFormatDescriptionGetStreamBasicDescription(d)?.pointee
            {
                if basic.mSampleRate > 0 {
                    parts.append(String(format: "%.4g kHz", basic.mSampleRate / 1000))
                }
                switch basic.mChannelsPerFrame {
                case 1: parts.append("Mono")
                case 2: parts.append("Stereo")
                case let n where n > 2: parts.append("\(n) ch")
                default: break
                }
            }
        }
        if let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize {
            parts.append(ByteCountFormatter.string(fromByteCount: Int64(size), countStyle: .file))
        }
        return parts.joined(separator: " · ")
    }

    /// Peak amplitude per bucket, read in chunks.
    ///
    /// Chunked because the alternative is decoding the whole file into one PCM
    /// buffer, and ten minutes of stereo 44.1 kHz float is over 200 MB to
    /// answer a question about a few hundred pixels. Runs off the main thread
    /// and returns empty for anything `AVAudioFile` will not open, which the
    /// view treats as "no waveform" rather than as an error.
    private nonisolated static func scanPeaks(_ url: URL, buckets: Int) -> [Float] {
        guard let file = try? AVAudioFile(forReading: url) else { return [] }
        let total = file.length
        guard total > 0, buckets > 0 else { return [] }
        let fmt = file.processingFormat
        let framesPerBucket = max(1, Int(total) / buckets)
        // Cap the read size so a very long file does not ask for a very large
        // buffer; several reads per bucket is fine.
        let chunkFrames = AVAudioFrameCount(min(framesPerBucket, 1 << 16))
        guard let buf = AVAudioPCMBuffer(pcmFormat: fmt, frameCapacity: chunkFrames) else {
            return []
        }

        var out: [Float] = []
        out.reserveCapacity(buckets)
        var bucketPeak: Float = 0
        var framesInBucket = 0

        while file.framePosition < total {
            if Task.isCancelled { return [] }
            do { try file.read(into: buf) } catch { break }
            let n = Int(buf.frameLength)
            if n == 0 { break }
            guard let channels = buf.floatChannelData else { break }
            // Left channel alone: a waveform is a shape, not a measurement, and
            // summing channels doubles the work to move the outline slightly.
            let samples = channels[0]
            var i = 0
            while i < n {
                let v = abs(samples[i])
                if v > bucketPeak { bucketPeak = v }
                i += 1
            }
            framesInBucket += n
            if framesInBucket >= framesPerBucket {
                out.append(bucketPeak)
                bucketPeak = 0
                framesInBucket = 0
                if out.count >= buckets { break }
            }
        }
        if framesInBucket > 0, out.count < buckets { out.append(bucketPeak) }

        // Normalise to the loudest peak so a quiet recording still fills the
        // strip. A file of pure silence would divide by zero, so it stays flat.
        let loudest = out.max() ?? 0
        guard loudest > 0.0001 else { return out.map { _ in 0 } }
        return out.map { $0 / loudest }
    }
}

// MARK: - View

struct AudioViewer: View {
    let path: String
    let palette: ViewerPalette

    @StateObject private var model = AudioPlayerModel()

    var body: some View {
        GeometryReader { geo in
            let compact = geo.size.height < 420
            VStack(spacing: 0) {
                Spacer(minLength: 12)
                artworkTile(side: artworkSide(in: geo.size))
                titleBlock
                    .padding(.top, compact ? 12 : 20)
                if !compact {
                    playButtons.padding(.top, 18)
                }
                Spacer(minLength: 12)
                transportCard(compact: compact)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.horizontal, 24)
            .padding(.bottom, 18)
        }
        .background(palette.bg)
        .task(id: path) { model.open(path) }
        .onDisappear { model.close() }
    }

    /// Square, and never taller than the space left after the text and the
    /// card. Music can assume a window; a pane can be any shape.
    private func artworkSide(in size: CGSize) -> CGFloat {
        let byHeight = size.height - 250
        return max(72, min(260, min(size.width - 80, byHeight)))
    }

    // MARK: Artwork

    private func artworkTile(side: CGFloat) -> some View {
        Group {
            if let art = model.artwork {
                Image(nsImage: art)
                    .resizable()
                    .interpolation(.high)
                    .aspectRatio(contentMode: .fill)
            } else {
                ZStack {
                    // Music's placeholder is a flat grey square with a note.
                    // Tinting it with the theme accent keeps a file with no
                    // cover art from being the one colourless thing on screen.
                    LinearGradient(
                        colors: [palette.accent.opacity(0.32), palette.accent.opacity(0.10)],
                        startPoint: .topLeading, endPoint: .bottomTrailing
                    )
                    Image(systemName: "music.note")
                        .font(.system(size: side * 0.3, weight: .light))
                        .foregroundStyle(palette.fg.opacity(0.55))
                }
            }
        }
        .frame(width: side, height: side)
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .shadow(color: .black.opacity(0.34), radius: 16, y: 7)
    }

    // MARK: Title block

    private var titleBlock: some View {
        VStack(spacing: 3) {
            Text(model.title)
                .font(.system(size: 19, weight: .bold))
                .foregroundStyle(palette.fg)
                .lineLimit(1)
                .truncationMode(.middle)
            if !model.artist.isEmpty {
                // The accent line. This is the single strongest thing Music's
                // album page does and it costs nothing to keep.
                Text(model.artist)
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(palette.accent)
                    .lineLimit(1)
            }
            if !subtitle.isEmpty {
                Text(subtitle)
                    .font(.system(size: 11))
                    .foregroundStyle(palette.dim)
                    .lineLimit(1)
                    .padding(.top, 2)
            }
        }
        .multilineTextAlignment(.center)
    }

    private var subtitle: String {
        [model.album, model.year].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    // MARK: Buttons

    private var playButtons: some View {
        HStack(spacing: 10) {
            capsule(
                model.playing ? "일시정지" : "재생",
                symbol: model.playing ? "pause.fill" : "play.fill",
                filled: true
            ) { model.toggle() }
            capsule("처음부터", symbol: "backward.end.fill", filled: false) {
                model.seek(to: 0)
            }
        }
    }

    private func capsule(
        _ title: String, symbol: String, filled: Bool, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 5) {
                Image(systemName: symbol).font(.system(size: 11, weight: .bold))
                Text(title).font(.system(size: 12.5, weight: .semibold))
            }
            .foregroundStyle(palette.accent)
            .frame(minWidth: 92)
            .padding(.vertical, 8)
            .background(
                Capsule().fill(palette.accent.opacity(filled ? 0.20 : 0.11))
            )
        }
        .buttonStyle(.plain)
    }

    // MARK: The floating card

    /// Music docks its transport in a rounded card that floats over the page
    /// rather than a bar welded to the window edge. Same here, and it is also
    /// what makes the waveform read as a control instead of as decoration.
    private func transportCard(compact: Bool) -> some View {
        VStack(spacing: 8) {
            HStack(spacing: 14) {
                iconButton("gobackward.10") { model.skip(-10) }
                iconButton(model.playing ? "pause.fill" : "play.fill", large: true) {
                    model.toggle()
                }
                iconButton("goforward.10") { model.skip(10) }
                WaveformStrip(
                    peaks: model.peaks,
                    progress: model.duration > 0 ? model.elapsed / model.duration : 0,
                    palette: palette
                ) { frac in
                    model.seek(to: frac * model.duration)
                }
                .frame(height: 34)
                Text(timeLabel)
                    .font(.system(size: 11, weight: .medium).monospacedDigit())
                    .foregroundStyle(palette.dim)
                    .layoutPriority(1)
            }
            if !compact, !model.format.isEmpty {
                Text(model.format)
                    .font(.system(size: 10))
                    .foregroundStyle(palette.dim.opacity(0.8))
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(palette.fg.opacity(0.07))
                .overlay(
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .strokeBorder(palette.fg.opacity(0.09), lineWidth: 1)
                )
        )
    }

    private func iconButton(
        _ symbol: String, large: Bool = false, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: large ? 19 : 14, weight: .medium))
                .foregroundStyle(palette.fg)
                .frame(width: large ? 26 : 20, height: 24)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private var timeLabel: String {
        "\(Self.clock(model.elapsed)) / \(Self.clock(model.duration))"
    }

    static func clock(_ s: Double) -> String {
        guard s.isFinite, s >= 0 else { return "0:00" }
        let t = Int(s.rounded(.down))
        return String(format: "%d:%02d", t / 60, t % 60)
    }
}

// MARK: - Waveform

/// The scan's buckets, averaged down to whatever the strip is actually wide,
/// drawn symmetrically about the middle. Played side in the accent, the rest
/// dim — the same read as Music's progress line, with the shape of the sound
/// in it.
private struct WaveformStrip: View {
    let peaks: [Float]
    let progress: Double
    let palette: ViewerPalette
    let seek: (Double) -> Void

    var body: some View {
        GeometryReader { geo in
            let w = max(1, geo.size.width)
            let h = geo.size.height
            Canvas { ctx, size in
                let barW: CGFloat = 2
                let gap: CGFloat = 1
                let n = max(1, Int(size.width / (barW + gap)))
                let mid = size.height / 2
                let playedX = size.width * CGFloat(min(max(0, progress), 1))
                for i in 0..<n {
                    let x = CGFloat(i) * (barW + gap)
                    let amp = CGFloat(bucket(i, of: n))
                    // A floor, so silence is still a visible track rather than
                    // a gap in the control.
                    let barH = max(2, amp * (size.height - 3))
                    let r = CGRect(x: x, y: mid - barH / 2, width: barW, height: barH)
                    ctx.fill(
                        Path(roundedRect: r, cornerRadius: 1),
                        with: .color(x + barW <= playedX
                            ? palette.accent
                            : palette.fg.opacity(0.22))
                    )
                }
                if peaks.isEmpty {
                    // No waveform available: a plain track and a played run,
                    // which is still a working scrubber.
                    let track = CGRect(x: 0, y: mid - 1.5, width: size.width, height: 3)
                    ctx.fill(Path(roundedRect: track, cornerRadius: 1.5),
                             with: .color(palette.fg.opacity(0.18)))
                    let done = CGRect(x: 0, y: mid - 1.5, width: playedX, height: 3)
                    ctx.fill(Path(roundedRect: done, cornerRadius: 1.5),
                             with: .color(palette.accent))
                }
            }
            .frame(height: h)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { g in seek(Double(min(max(0, g.location.x), w) / w)) }
            )
        }
    }

    /// Bucket `i` of `n`, averaged from however many scan buckets fall in it.
    private func bucket(_ i: Int, of n: Int) -> Float {
        guard !peaks.isEmpty else { return 0 }
        let lo = peaks.count * i / n
        let hi = max(lo + 1, peaks.count * (i + 1) / n)
        guard lo < peaks.count else { return 0 }
        let slice = peaks[lo..<min(hi, peaks.count)]
        return slice.reduce(0, +) / Float(slice.count)
    }
}
