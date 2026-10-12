import AudioToolbox
import AVFoundation

/// Mac의 소리를 재생합니다. AAC-ELD를 AudioQueue에 넣어 시스템 디코더로 풀어 냅니다.
///
/// - 세션의 사건 스레드에서만 부르십시오. 영상과 시각을 맞추지 않고 받는 대로 재생합니다.
/// - 설정(AudioSpecificConfig)을 매직 쿠키로 넘겨 표본 수와 채널을 시스템이 정하게 합니다.
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
