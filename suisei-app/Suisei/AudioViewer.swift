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
    /// 0…1, this player's own level — not the system's. Persisted, because a
    /// volume that resets every time a file is opened is not a setting.
    @Published var volume: Float = AudioPlayerModel.storedVolume {
        didSet {
            player?.volume = muted ? 0 : volume
            UserDefaults.standard.set(Double(volume), forKey: Self.volumeKey)
        }
    }
    @Published var muted = false {
        didSet { player?.volume = muted ? 0 : volume }
    }

    private static let volumeKey = "suisei.audio.volume"
    private static var storedVolume: Float {
        // `object(forKey:)` first: `float(forKey:)` answers 0 for a key that
        // was never written, which would open every first session silent.
        guard let v = UserDefaults.standard.object(forKey: volumeKey) as? Double else {
            return 1
        }
        return Float(min(max(0, v), 1))
    }
    /// Peak amplitude per horizontal bucket, 0…1. Empty until the scan lands,
    /// and empty forever for a file `AVAudioFile` cannot open — the scrubber
    /// falls back to a plain bar rather than the view failing.
    @Published var peaks: [Float] = []
    @Published var format = ""
    /// The inspector's contents. Built once when the file loads; the panel is
    /// a read-only view of a file that is not changing.
    @Published var sections: [InfoSection] = []

    struct InfoRow: Identifiable {
        var id: String { label }
        let label: String
        let value: String
    }

    struct InfoSection: Identifiable {
        var id: String { title }
        let title: String
        let rows: [InfoRow]
    }

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
        player.volume = muted ? 0 : volume
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
        sections = []
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
        // Every metadata format the container carries, not just the common
        // keys: an MP3's ID3 has a genre, a track number and a composer that
        // `commonMetadata` drops, and a pane in a development environment is
        // the place to show them.
        var tags: [String: String] = [:]
        if let items = try? await asset.load(.metadata) {
            for item in items {
                if item.commonKey == .commonKeyArtwork {
                    if artwork == nil, let d = try? await item.load(.dataValue) {
                        artwork = NSImage(data: d)
                    }
                    continue
                }
                guard let name = Self.tagName(item) else { continue }
                guard tags[name] == nil else { continue }
                if let s = try? await item.load(.stringValue), !s.isEmpty {
                    tags[name] = s
                } else if let n = try? await item.load(.numberValue) {
                    tags[name] = n.stringValue
                }
            }
        }
        if let s = tags["Title"], !s.isEmpty { title = s }
        artist = tags["Artist"] ?? ""
        album = tags["Album"] ?? ""
        year = tags["Year"].map { String($0.prefix(4)) } ?? ""

        let audio = await Self.audioFacts(asset)
        format = Self.oneLineFormat(audio, url: url)
        sections = Self.buildSections(audio, tags: tags, url: url, duration: duration)
    }

    /// What the container says about the audio stream itself.
    private struct AudioFacts {
        var codec = ""
        var sampleRate: Double = 0
        var bitDepth: UInt32 = 0
        var channels: UInt32 = 0
        var bitrate: Float = 0
    }

    private static func audioFacts(_ asset: AVURLAsset) async -> AudioFacts {
        var f = AudioFacts()
        guard let track = try? await asset.loadTracks(withMediaType: .audio).first else {
            return f
        }
        if let rate = try? await track.load(.estimatedDataRate) { f.bitrate = rate }
        if let descs = try? await track.load(.formatDescriptions),
           let d = descs.first,
           let basic = CMAudioFormatDescriptionGetStreamBasicDescription(d)?.pointee
        {
            f.codec = codecName(basic.mFormatID)
            f.sampleRate = basic.mSampleRate
            f.bitDepth = basic.mBitsPerChannel
            f.channels = basic.mChannelsPerFrame
        }
        return f
    }

    /// `MP3 · 173 kbps · 48 kHz · Stereo · 3.6 MB` — the compact line under
    /// the transport, for when the pane is too narrow for the inspector.
    private static func oneLineFormat(_ f: AudioFacts, url: URL) -> String {
        var parts: [String] = []
        if !f.codec.isEmpty { parts.append(f.codec) }
        if f.bitrate > 0 { parts.append("\(Int((f.bitrate / 1000).rounded())) kbps") }
        if f.sampleRate > 0 { parts.append(shortHz(f.sampleRate)) }
        if let c = channelName(f.channels) { parts.append(c) }
        if let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize {
            parts.append(ByteCountFormatter.string(fromByteCount: Int64(size), countStyle: .file))
        }
        return parts.joined(separator: " · ")
    }

    private static func buildSections(
        _ f: AudioFacts, tags: [String: String], url: URL, duration: Double
    ) -> [InfoSection] {
        var out: [InfoSection] = []

        var audio: [InfoRow] = []
        if !f.codec.isEmpty { audio.append(.init(label: "Codec", value: f.codec)) }
        if duration > 0 {
            audio.append(.init(label: "Duration", value: AudioViewer.clock(duration)))
        }
        if f.sampleRate > 0 {
            // Exact, with separators. `48 kHz` is the label a music player
            // uses; a development environment wants to see 48,000.
            audio.append(.init(label: "Sample Rate", value: "\(grouped(Int(f.sampleRate))) Hz"))
        }
        if f.bitDepth > 0 {
            audio.append(.init(label: "Bit Depth", value: "\(f.bitDepth)-bit"))
        }
        if let c = channelName(f.channels) {
            audio.append(.init(label: "Channels", value: "\(c) (\(f.channels))"))
        }
        if f.bitrate > 0 {
            audio.append(.init(label: "Bit Rate", value: "\(grouped(Int(f.bitrate / 1000))) kbps"))
        }
        if !audio.isEmpty { out.append(.init(title: "Audio", rows: audio)) }

        var file: [InfoRow] = []
        let values = try? url.resourceValues(forKeys: [
            .fileSizeKey, .contentTypeKey, .creationDateKey, .contentModificationDateKey,
        ])
        if let t = values?.contentType?.localizedDescription {
            file.append(.init(label: "Kind", value: t))
        }
        if let s = values?.fileSize {
            file.append(.init(
                label: "Size",
                value: ByteCountFormatter.string(fromByteCount: Int64(s), countStyle: .file)
            ))
        }
        if let d = values?.creationDate {
            file.append(.init(label: "Created", value: stamp(d)))
        }
        if let d = values?.contentModificationDate {
            file.append(.init(label: "Modified", value: stamp(d)))
        }
        if !file.isEmpty { out.append(.init(title: "File", rows: file)) }

        // Tags in a fixed order — a dictionary's order is not one, and a panel
        // whose rows move between files is unreadable.
        let order = ["Title", "Artist", "Album", "Album Artist", "Year", "Genre",
                     "Track", "Disc", "Composer", "Encoder", "Comment"]
        var meta: [InfoRow] = order.compactMap { k in
            guard let v = tags[k], !v.isEmpty else { return nil }
            return InfoRow(label: k, value: v)
        }
        for (k, v) in tags.sorted(by: { $0.key < $1.key })
        where !order.contains(k) && !v.isEmpty {
            meta.append(.init(label: k, value: v))
        }
        if !meta.isEmpty { out.append(.init(title: "Metadata", rows: meta)) }

        return out
    }

    // MARK: Small formatters

    private static func codecName(_ id: AudioFormatID) -> String {
        switch id {
        case kAudioFormatLinearPCM: return "Linear PCM"
        case kAudioFormatMPEGLayer3: return "MP3"
        case kAudioFormatMPEGLayer2: return "MP2"
        case kAudioFormatMPEG4AAC: return "AAC"
        case kAudioFormatMPEG4AAC_HE: return "HE-AAC"
        case kAudioFormatMPEG4AAC_HE_V2: return "HE-AAC v2"
        case kAudioFormatAppleLossless: return "Apple Lossless"
        case kAudioFormatFLAC: return "FLAC"
        case kAudioFormatOpus: return "Opus"
        case kAudioFormatAppleIMA4: return "IMA 4:1"
        case kAudioFormatALaw: return "A-law"
        case kAudioFormatULaw: return "µ-law"
        default:
            // Anything unnamed still says something useful: the four-character
            // code is what the format actually is.
            let b = [24, 16, 8, 0].map { UInt8((id >> UInt32($0)) & 0xFF) }
            let s = String(bytes: b, encoding: .ascii)?
                .trimmingCharacters(in: .whitespaces) ?? ""
            return s.isEmpty ? "Unknown" : s
        }
    }

    private static func channelName(_ n: UInt32) -> String? {
        switch n {
        case 0: return nil
        case 1: return "Mono"
        case 2: return "Stereo"
        default: return "\(n) channels"
        }
    }

    private static func shortHz(_ hz: Double) -> String {
        String(format: "%.4g kHz", hz / 1000)
    }

    private static func grouped(_ n: Int) -> String {
        let f = NumberFormatter()
        f.numberStyle = .decimal
        return f.string(from: NSNumber(value: n)) ?? "\(n)"
    }

    private static func stamp(_ d: Date) -> String {
        let f = DateFormatter()
        f.dateStyle = .medium
        f.timeStyle = .short
        return f.string(from: d)
    }

    /// A readable name for a metadata item, from whichever of the several tag
    /// vocabularies the container happens to use.
    private static func tagName(_ item: AVMetadataItem) -> String? {
        if let common = item.commonKey {
            switch common {
            case .commonKeyTitle: return "Title"
            case .commonKeyArtist, .commonKeyCreator: return "Artist"
            case .commonKeyAlbumName: return "Album"
            case .commonKeyCreationDate: return "Year"
            case .commonKeyType: return "Genre"
            case .commonKeyDescription: return "Comment"
            case .commonKeySoftware: return "Encoder"
            case .commonKeyAuthor: return "Composer"
            case .commonKeyPublisher: return "Publisher"
            default: return nil
            }
        }
        // Not a common key: fall back to the raw key, which for ID3 and iTunes
        // atoms is a short code.
        guard let raw = item.key as? String else { return nil }
        switch raw {
        case "TRCK", "trkn", "TRK": return "Track"
        case "TPOS", "disk": return "Disc"
        case "TPE2", "aART": return "Album Artist"
        case "TCOM", "©wrt": return "Composer"
        // `TSSE` is what ffmpeg writes, and it was the encoder line this file
        // actually had — mapping only `TENC` found nothing.
        case "TENC", "TSSE", "©too": return "Encoder"
        case "TCON", "©gen": return "Genre"
        // Likewise the year: this file carries `TYER` and no common creation
        // date, so the common key alone left the year blank.
        case "TYER", "TDRC", "TDRL", "TDAT", "©day": return "Year"
        case "COMM", "©cmt": return "Comment"
        case "TIT2": return "Title"
        case "TPE1": return "Artist"
        case "TALB": return "Album"
        case "TBPM": return "BPM"
        case "TPUB": return "Publisher"
        case "TXXX":
            // A user-defined frame. Its real name is in the extra attributes;
            // without it every one of them would be called "TXXX" and all but
            // the first would be dropped as a duplicate.
            let name = (item.extraAttributes?[.info] as? String)?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            return (name?.isEmpty == false) ? name : "Comment"
        default: return nil
        }
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

    /// How big the hero block is drawn — artwork, title, artist, year and the
    /// two buttons, all off this one number so they grow as one thing.
    ///
    /// Persisted: it is a preference about how the user wants to look at audio
    /// files, not about this file.
    @AppStorage("suisei.audio.heroScale") private var heroScale: Double = 1
    /// The scale when the current drag began. `nil` between drags — the
    /// gesture reports a cumulative translation, so it needs the value it
    /// started from rather than the value it last produced.
    @State private var dragBase: Double?
    @State private var resizeHovering = false

    /// Below this the inspector is dropped rather than squeezed — a two-column
    /// layout in a narrow split pane gives neither column enough room.
    private static let inspectorMinWidth: CGFloat = 620
    private static let inspectorWidth: CGFloat = 230
    private static let minScale: Double = 0.55
    private static let maxScale: Double = 1.85
    /// Points of vertical drag for one unit of scale. Tuned so the whole range
    /// is a comfortable single gesture rather than a sweep of the screen.
    private static let dragPerScale: Double = 280

    /// A hero-block measurement at the current scale.
    private func s(_ v: CGFloat) -> CGFloat { v * heroScale }

    var body: some View {
        GeometryReader { geo in
            let compact = geo.size.height < 420
            let showInspector = geo.size.width >= Self.inspectorMinWidth
                && !model.sections.isEmpty
            VStack(spacing: 0) {
                HStack(alignment: .top, spacing: 0) {
                    VStack(spacing: 0) {
                        Spacer(minLength: 12)
                        artworkTile(side: artworkSide(in: geo.size, inspector: showInspector))
                        titleBlock
                            .padding(.top, s(compact ? 12 : 20))
                        if !compact {
                            playButtons.padding(.top, s(18))
                        }
                        Spacer(minLength: 12)
                    }
                    .frame(maxWidth: .infinity)
                    // The whole column is the grip, so the drag can start on
                    // the artwork or on the empty space beside it — wherever
                    // the pointer happens to be.
                    .contentShape(Rectangle())
                    .gesture(heroResize)
                    // The cursor is the only thing telling anyone this is
                    // draggable; nothing else on the pane would suggest it.
                    //
                    // Push and pop have to balance exactly. `NSCursor` keeps a
                    // stack, and one unmatched push leaves the resize cursor
                    // over the entire app until something else pushes — so the
                    // flag gates both edges rather than the callback being
                    // trusted to alternate.
                    .onHover { inside in
                        guard inside != resizeHovering else { return }
                        resizeHovering = inside
                        if inside { NSCursor.resizeUpDown.push() } else { NSCursor.pop() }
                    }
                    .onDisappear {
                        if resizeHovering {
                            resizeHovering = false
                            NSCursor.pop()
                        }
                    }
                    if showInspector {
                        Divider().overlay(palette.fg.opacity(0.10))
                        InfoInspector(sections: model.sections, palette: palette)
                            .frame(width: Self.inspectorWidth)
                    }
                }
                transportCard(
                    compact: compact,
                    showFormatLine: !showInspector,
                    // The first thing to go when the card runs out of room:
                    // the waveform is the control that has to survive, and a
                    // squeezed slider is worse than no slider.
                    showVolume: geo.size.width >= 520
                )
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.horizontal, 24)
            .padding(.bottom, 18)
        }
        .background(palette.bg)
        .task(id: path) { model.open(path) }
        .onDisappear { model.close() }
    }

    /// Drag anywhere in the hero column to resize it.
    ///
    /// Down is bigger, which is the direction the block grows. `minimumDistance`
    /// is deliberately non-zero: at zero the gesture claims the press before a
    /// child can, and the play buttons underneath stop working.
    private var heroResize: some Gesture {
        DragGesture(minimumDistance: 4)
            .onChanged { g in
                let base = dragBase ?? heroScale
                if dragBase == nil { dragBase = base }
                heroScale = min(
                    Self.maxScale,
                    max(Self.minScale, base + Double(g.translation.height) / Self.dragPerScale)
                )
            }
            .onEnded { _ in dragBase = nil }
    }

    /// Never taller than the space left after the text and the card, and never
    /// wider than the column it sits in. Music can assume a window; a pane can
    /// be any shape, the inspector takes a bite out of the width, and the user
    /// can now ask for a size the pane cannot give.
    ///
    /// The scale is a request, not an instruction: the clamps still win, so
    /// dragging past what fits stops growing rather than pushing the transport
    /// off the bottom.
    private func artworkSide(in size: CGSize, inspector: Bool) -> CGFloat {
        let column = size.width - (inspector ? Self.inspectorWidth + 24 : 0)
        // The text and buttons scale with the artwork; the transport card does
        // not, so only part of the reservation moves.
        let byHeight = size.height - (s(150) + 110)
        return max(56, min(s(260), min(column - s(80), byHeight)))
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
        .frame(width: tileSize(side).width, height: tileSize(side).height)
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .shadow(color: .black.opacity(0.34), radius: 16, y: 7)
    }

    /// The tile takes the artwork's own shape, bounded by `side`.
    ///
    /// A square frame with `.fill` cropped this cover's left and right edges
    /// off — it is wider than it is tall, and half the text printed on the
    /// sleeve went with them. `.fit` inside a square frame is not the fix
    /// either: the image would letterbox, and the rounded corners and the
    /// shadow would be drawn around the empty bands rather than around the
    /// picture. Sizing the frame to the aspect makes fill and fit the same
    /// thing and leaves nothing to crop.
    private func tileSize(_ side: CGFloat) -> CGSize {
        guard let art = model.artwork, art.size.width > 0, art.size.height > 0 else {
            return CGSize(width: side, height: side)
        }
        let ar = art.size.width / art.size.height
        return ar >= 1
            ? CGSize(width: side, height: (side / ar).rounded())
            : CGSize(width: (side * ar).rounded(), height: side)
    }

    // MARK: Title block

    private var titleBlock: some View {
        VStack(spacing: s(3)) {
            Text(model.title)
                .font(.system(size: s(19), weight: .bold))
                .foregroundStyle(palette.fg)
                .lineLimit(1)
                .truncationMode(.middle)
            if !model.artist.isEmpty {
                // The accent line. This is the single strongest thing Music's
                // album page does and it costs nothing to keep.
                Text(model.artist)
                    .font(.system(size: s(16), weight: .semibold))
                    .foregroundStyle(palette.accent)
                    .lineLimit(1)
            }
            if !subtitle.isEmpty {
                Text(subtitle)
                    .font(.system(size: s(11)))
                    .foregroundStyle(palette.dim)
                    .lineLimit(1)
                    .padding(.top, s(2))
            }
        }
        .multilineTextAlignment(.center)
    }

    private var subtitle: String {
        [model.album, model.year].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    // MARK: Buttons

    private var playButtons: some View {
        HStack(spacing: s(10)) {
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
            HStack(spacing: s(5)) {
                Image(systemName: symbol).font(.system(size: s(11), weight: .bold))
                Text(title).font(.system(size: s(12.5), weight: .semibold))
            }
            .foregroundStyle(palette.accent)
            .frame(minWidth: s(92))
            .padding(.vertical, s(8))
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
    private func transportCard(
        compact: Bool, showFormatLine: Bool, showVolume: Bool
    ) -> some View {
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
                // `fixedSize`, or the clock loses its tail. The waveform is a
                // GeometryReader, which has no intrinsic width and takes every
                // point offered — the label was the only thing left that could
                // give, so it did, and `0:01 / 2:00` rendered as `0:01 / 2`.
                Text(timeLabel)
                    .font(.system(size: 11, weight: .medium).monospacedDigit())
                    .foregroundStyle(palette.dim)
                    .fixedSize()
                if showVolume {
                    VolumeControl(
                        volume: $model.volume,
                        muted: $model.muted,
                        palette: palette
                    )
                    .fixedSize()
                }
            }
            if !compact, showFormatLine, !model.format.isEmpty {
                Text(model.format)
                    .font(.system(size: 10))
                    .foregroundStyle(palette.dim.opacity(0.8))
                    .lineLimit(1)
            }
        }
        // Wider than it looks like it needs: a capsule's end is a half-circle,
        // so at this height the first 28pt of each side is curve. Content laid
        // out to 20 sat inside it.
        .padding(.horizontal, 28)
        .padding(.vertical, 12)
        // The toolbar's material, not an imitation of it.
        //
        // The three buttons at the top right look the way they do because
        // macOS 26 wraps a toolbar item in an `NSGlassEffectView` — see
        // `editorToolbar`, where the whole point is that the app draws nothing.
        // `glassEffect` is the same material from the SwiftUI side, so this
        // card is made of what the toolbar is made of rather than of a fill
        // and a hairline chosen to resemble it. A capsule is that API's own
        // default shape, which is also the shape asked for.
        .glassEffect(.regular, in: Capsule())
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

// MARK: - Volume

/// A speaker that mutes when clicked, and a slim track beside it.
///
/// Hand-drawn rather than a SwiftUI `Slider` for the same reason the buttons
/// are: `Slider` paints itself in the system accent, which is the one colour
/// on this pane that has nothing to do with the theme the user chose. It also
/// arrives at a size decided by AppKit, and next to a 3pt waveform that reads
/// as a piece of a different application.
///
/// This is the player's own level, not the system's — turning it down here
/// does not reach anything else that is making sound.
private struct VolumeControl: View {
    @Binding var volume: Float
    @Binding var muted: Bool
    let palette: ViewerPalette

    private static let trackWidth: CGFloat = 58

    var body: some View {
        HStack(spacing: 7) {
            Button { muted.toggle() } label: {
                Image(systemName: symbol)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(muted ? palette.dim : palette.fg)
                    // A fixed box: the wave glyphs are different widths, and
                    // without it the track shifts sideways as the level moves.
                    .frame(width: 15, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(muted ? "음소거 해제" : "음소거")

            GeometryReader { geo in
                let w = max(1, geo.size.width)
                let filled = muted ? 0 : CGFloat(volume) * w
                ZStack(alignment: .leading) {
                    Capsule()
                        .fill(palette.fg.opacity(0.20))
                        .frame(height: 3)
                    Capsule()
                        .fill(muted ? palette.fg.opacity(0.28) : palette.accent)
                        .frame(width: filled, height: 3)
                    Circle()
                        .fill(muted ? palette.fg.opacity(0.35) : palette.accent)
                        .frame(width: 8, height: 8)
                        // Inset by the knob's own radius so it stops at the
                        // ends of the track instead of hanging off them.
                        .offset(x: min(max(0, filled - 4), w - 8))
                }
                .frame(maxHeight: .infinity)
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 0).onChanged { g in
                        // Dragging is also how you come back from muted —
                        // otherwise the slider looks live and does nothing.
                        muted = false
                        volume = Float(min(max(0, g.location.x / w), 1))
                    }
                )
            }
            .frame(width: Self.trackWidth, height: 16)
        }
    }

    private var symbol: String {
        if muted || volume <= 0.001 { return "speaker.slash.fill" }
        if volume < 0.34 { return "speaker.wave.1.fill" }
        if volume < 0.67 { return "speaker.wave.2.fill" }
        return "speaker.wave.3.fill"
    }
}

// MARK: - Inspector

/// What the file is, listed.
///
/// Apple's inspector shape — Preview's Info window, Music's Get Info: a
/// section title in small dim caps, then rows of a dim left label against a
/// right-aligned value in the document's ink. No separators, no boxes, no
/// alternating fill. The alignment does the work a rule would do, and the
/// panel stays quiet enough to sit beside the artwork without competing.
private struct InfoInspector: View {
    let sections: [AudioPlayerModel.InfoSection]
    let palette: ViewerPalette

    var body: some View {
        ScrollView(.vertical) {
            VStack(alignment: .leading, spacing: 18) {
                ForEach(sections) { section in
                    VStack(alignment: .leading, spacing: 6) {
                        Text(section.title.uppercased())
                            .font(.system(size: 9.5, weight: .semibold))
                            .tracking(0.6)
                            .foregroundStyle(palette.dim.opacity(0.75))
                        VStack(alignment: .leading, spacing: 4) {
                            ForEach(section.rows) { row in
                                if Self.isBlock(row.value) {
                                    // A credits list or a URL dump does not
                                    // belong in a right-aligned column — it
                                    // reads as ragged noise there. Same shape
                                    // as Get Info's Comments box: the label,
                                    // then the text under it, full width.
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(row.label)
                                            .font(.system(size: 11))
                                            .foregroundStyle(palette.dim)
                                        Text(row.value)
                                            .font(.system(size: 10.5))
                                            .foregroundStyle(palette.fg.opacity(0.9))
                                            .fixedSize(horizontal: false, vertical: true)
                                            .textSelection(.enabled)
                                    }
                                    .padding(.top, 2)
                                } else {
                                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                                        Text(row.label)
                                            .font(.system(size: 11))
                                            .foregroundStyle(palette.dim)
                                        Spacer(minLength: 6)
                                        Text(row.value)
                                            .font(.system(size: 11, weight: .medium))
                                            .foregroundStyle(palette.fg)
                                            .multilineTextAlignment(.trailing)
                                            // Wraps rather than truncates: a
                                            // long album title is exactly what
                                            // someone opened this panel to read.
                                            .fixedSize(horizontal: false, vertical: true)
                                            .textSelection(.enabled)
                                    }
                                }
                            }
                        }
                    }
                }
            }
            .padding(.horizontal, 18)
            .padding(.vertical, 20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.never)
    }

    /// Too long or too many lines to sit in the value column.
    private static func isBlock(_ v: String) -> Bool {
        v.contains("\n") || v.count > 42
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
