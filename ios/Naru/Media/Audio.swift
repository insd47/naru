import AudioToolbox
import AVFoundation

/// Plays the Mac's audio by feeding AAC-ELD to an AudioQueue, which decodes it with the system decoder.
///
/// - Call only from the session's event thread. Audio plays as it arrives, without syncing to video.
/// - The AudioSpecificConfig is passed as the magic cookie so the system determines the sample rate and channels.
final class Audio: @unchecked Sendable {
    private let queue: AudioQueueRef

    init?(config: Data) {
        var format = AudioStreamBasicDescription()
        format.mFormatID = kAudioFormatMPEG4AAC_ELD_SBR
        var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)

        let described = config.withUnsafeBytes {
            AudioFormatGetProperty(kAudioFormatProperty_FormatInfo, UInt32(config.count), $0.baseAddress, &size, &format)
        }

        var queue: AudioQueueRef?
        guard described == noErr, AudioQueueNewOutput(&format, { _, queue, buffer in AudioQueueFreeBuffer(queue, buffer) }, nil, nil, nil, 0, &queue) == noErr, let queue else {
            return nil
        }

        self.queue = queue
        _ = config.withUnsafeBytes { AudioQueueSetProperty(queue, kAudioQueueProperty_MagicCookie, $0.baseAddress!, UInt32(config.count)) }

        try? AVAudioSession.sharedInstance().setCategory(.playback, options: .mixWithOthers)
        try? AVAudioSession.sharedInstance().setActive(true)
        AudioQueueStart(queue, nil)
    }

    deinit {
        AudioQueueDispose(queue, true)
    }

    func play(_ frame: AudioFrame) {
        let data = frame.data
        var buffer: AudioQueueBufferRef?

        guard AudioQueueAllocateBufferWithPacketDescriptions(queue, UInt32(data.count), 1, &buffer) == noErr, let buffer else {
            return
        }

        data.withUnsafeBytes { buffer.pointee.mAudioData.copyMemory(from: $0.baseAddress!, byteCount: data.count) }
        buffer.pointee.mAudioDataByteSize = UInt32(data.count)

        var packet = AudioStreamPacketDescription(mStartOffset: 0, mVariableFramesInPacket: 0, mDataByteSize: UInt32(data.count))
        AudioQueueEnqueueBuffer(queue, buffer, 1, &packet)
    }
}
