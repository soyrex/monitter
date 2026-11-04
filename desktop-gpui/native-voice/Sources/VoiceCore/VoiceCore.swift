import Foundation
import AVFoundation
import CoreML
import CoreAudio
import FluidAudio
import PhononTTS
import Darwin

private func traceVoice(_ stage:String) {
    guard ProcessInfo.processInfo.environment["MONITTER_VOICE_DIAGNOSTICS"] == "1" else { return }
    let line="voice-stage \(ProcessInfo.processInfo.systemUptime) \(stage)\n"
    FileHandle.standardError.write(Data(line.utf8))
}

struct AudioRouteSnapshot: Equatable {
    let inputDevice:AudioDeviceID, outputDevice:AudioDeviceID
    let inputRate:Double, inputChannels:AVAudioChannelCount
    let outputRate:Double, outputChannels:AVAudioChannelCount
}

// An old configuration notification can arrive after startup. Only dismiss it
// when the engine is still running and both device identities and formats match.
func routeNotificationRequiresStop(running:Bool, expected:AudioRouteSnapshot?, current:AudioRouteSnapshot?) -> Bool {
    guard running, let expected, let current else { return true }
    return expected != current
}

private func defaultAudioDevice(_ selector:AudioObjectPropertySelector) -> AudioDeviceID? {
    var address=AudioObjectPropertyAddress(mSelector:selector,mScope:kAudioObjectPropertyScopeGlobal,mElement:kAudioObjectPropertyElementMain)
    var device=AudioDeviceID(kAudioObjectUnknown), size=UInt32(MemoryLayout<AudioDeviceID>.size)
    guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject),&address,0,nil,&size,&device) == noErr, device != kAudioObjectUnknown else { return nil }
    return device
}

private func audioRouteSnapshot(_ engine:AVAudioEngine) -> AudioRouteSnapshot? {
    guard let input=defaultAudioDevice(kAudioHardwarePropertyDefaultInputDevice), let output=defaultAudioDevice(kAudioHardwarePropertyDefaultOutputDevice) else { return nil }
    let i=engine.inputNode.inputFormat(forBus:0), o=engine.outputNode.outputFormat(forBus:0)
    guard i.sampleRate > 0, i.channelCount > 0, o.sampleRate > 0, o.channelCount > 0 else { return nil }
    return AudioRouteSnapshot(inputDevice:input,outputDevice:output,inputRate:i.sampleRate,inputChannels:i.channelCount,outputRate:o.sampleRate,outputChannels:o.channelCount)
}

public struct Command: Decodable {
    public let type: String, sessionId: String, epoch: String, requestId: String
    public let generation: UInt64?
    public let text: String?
    public let muted: Bool?
    public let modelsPath: String?
    public let testOnlyNoAudio: Bool?
}

public final class EventWriter: @unchecked Sendable {
    private let lock = NSLock()
    public init() {}
    public func send(_ event: [String: Any]) {
        lock.lock(); defer { lock.unlock() }
        guard JSONSerialization.isValidJSONObject(event), let data = try? JSONSerialization.data(withJSONObject: event, options: [.sortedKeys]) else { return }
        FileHandle.standardOutput.write(data); FileHandle.standardOutput.write(Data([10]))
    }
}

// Bound both queued audio and wall-clock lag. A transient model stall can
// drain in order; sustained overload fails closed rather than dropping speech.
final class InputQueue: @unchecked Sendable {
    private struct Entry { let buffer:AVAudioPCMBuffer; let duration:TimeInterval; let bytes:Int; let arrived:TimeInterval }
    struct OverflowDetails {
        let reason:String, queuedSeconds:Double, oldestSeconds:Double, count:Int, bytes:Int, frames:UInt32, rate:Double, channels:UInt32
        var diagnostic:String { "input-overflow reason=\(reason) queuedSeconds=\(queuedSeconds) oldestSeconds=\(oldestSeconds) count=\(count) bytes=\(bytes) frames=\(frames) rate=\(rate) channels=\(channels)" }
    }
    private static let maxDuration:TimeInterval = 4
    private static let maxAge:TimeInterval = 4
    private static let maxBytes = 8 * 1024 * 1024
    private static let maxBuffers = 1024
    private let lock = NSLock()
    private var buffers: [Entry] = []
    private var queuedDuration:TimeInterval = 0; private var queuedBytes = 0
    private var overflow = false; private var muted = false; private var accepting = true
    private var overflowInfo:OverflowDetails?
    private var lastFrames:UInt32=0; private var lastRate:Double=0; private var lastChannels:UInt32=0
    private var generation:UInt64 = 0; private var observedTaps:UInt64=0; private var acceptedTaps:UInt64=0; private var lastTapAt:TimeInterval=0
    func captureToken() -> UInt64? { lock.lock(); defer { lock.unlock() }; observedTaps &+= 1; lastTapAt=ProcessInfo.processInfo.systemUptime; guard !muted, !overflow, accepting else { return nil }; acceptedTaps &+= 1; return generation }
    func isCurrent(_ token:UInt64) -> Bool { lock.lock(); defer { lock.unlock() }; return !muted && !overflow && token == generation }
    func push(_ buffer: AVAudioPCMBuffer, now:TimeInterval = ProcessInfo.processInfo.systemUptime, token:UInt64? = nil) {
        lock.lock(); defer { lock.unlock() }
        guard !muted, !overflow, accepting, token == nil || token == generation else { return }
        let rate=buffer.format.sampleRate
        let duration=Double(buffer.frameLength)/rate
        let channels=buffer.format.isInterleaved ? 1 : Int(buffer.format.channelCount)
        let bytes=Int(buffer.frameCapacity) * Int(buffer.format.streamDescription.pointee.mBytesPerFrame) * channels
        lastFrames=buffer.frameLength; lastRate=rate.isFinite ? rate : 0; lastChannels=buffer.format.channelCount
        let reason:String?
        if !rate.isFinite || rate <= 0 || !duration.isFinite || duration <= 0 || bytes <= 0 { reason="invalidFormat" }
        else if buffers.count >= Self.maxBuffers { reason="count" }
        else if bytes > Self.maxBytes - queuedBytes { reason="bytes" }
        else if queuedDuration + duration > Self.maxDuration { reason="duration" }
        else if buffers.first.map({ now - $0.arrived >= Self.maxAge }) ?? false { reason="age" }
        else { reason=nil }
        if let reason { markOverflow(reason:reason,now:now); return }
        buffers.append(Entry(buffer:buffer,duration:duration,bytes:bytes,arrived:now))
        queuedDuration += duration; queuedBytes += bytes
    }
    func take(now:TimeInterval = ProcessInfo.processInfo.systemUptime) -> (AVAudioPCMBuffer?, Bool) {
        lock.lock(); defer { lock.unlock() }
        if let oldest=buffers.first, now - oldest.arrived >= Self.maxAge { markOverflow(reason:"age",now:now) }
        guard !overflow else { return (nil,true) }
        guard !buffers.isEmpty else { return (nil,false) }
        let entry=buffers.removeFirst(); queuedDuration=max(0,queuedDuration-entry.duration); queuedBytes-=entry.bytes
        return (entry.buffer,false)
    }
    private func markOverflow(reason:String, now:TimeInterval) {
        guard !overflow else { return }
        overflow=true
        overflowInfo=OverflowDetails(reason:reason,queuedSeconds:queuedDuration,oldestSeconds:max(0,now-(buffers.first?.arrived ?? now)),count:buffers.count,bytes:queuedBytes,frames:lastFrames,rate:lastRate,channels:lastChannels)
    }
    func lastTapTime() -> TimeInterval { lock.lock(); defer { lock.unlock() }; return lastTapAt }
    func tapMetrics() -> (UInt64,UInt64) { lock.lock(); defer { lock.unlock() }; return (observedTaps,acceptedTaps) }
    func overflowDetails() -> OverflowDetails? { lock.lock(); defer { lock.unlock() }; return overflowInfo }
    func metrics(now:TimeInterval = ProcessInfo.processInfo.systemUptime) -> (rate:Double,frames:UInt32,queued:Double,age:Double,count:Int) {
        lock.lock(); defer { lock.unlock() }
        return (lastRate,lastFrames,queuedDuration,max(0,buffers.first.map { now-$0.arrived } ?? 0),buffers.count)
    }
    private func reset() { buffers.removeAll(); queuedDuration=0; queuedBytes=0; overflow=false; overflowInfo=nil; lastFrames=0; lastRate=0; lastChannels=0; generation &+= 1 }
    func clear() { lock.lock(); reset(); lock.unlock() }
    func setMuted(_ value: Bool) { lock.lock(); muted=value; accepting = !value; if !value { lastTapAt=ProcessInfo.processInfo.systemUptime }; reset(); lock.unlock() }
    func freeze() { lock.lock(); accepting=false; lock.unlock() }
}

// Feed FluidAudio its native format, avoiding a per-buffer converter loop that
// has no no-progress exit. One conversion has a bounded output allocation.
func normalizeInput(_ buffer: AVAudioPCMBuffer) throws -> AVAudioPCMBuffer {
    let target=AVAudioFormat(standardFormatWithSampleRate:16000,channels:1)!
    if buffer.format == target { return buffer }
    guard let converter=AVAudioConverter(from:buffer.format,to:target) else { throw VoiceError("cannot convert microphone format") }
    let count=ceil(Double(buffer.frameLength)*16000/buffer.format.sampleRate)+4096
    guard count.isFinite, count > 0, count <= Double(UInt32.max),
          let out=AVAudioPCMBuffer(pcmFormat:target,frameCapacity:AVAudioFrameCount(count)) else { throw VoiceError("invalid microphone buffer size") }
    var supplied=false, conversionError:NSError?
    let status=converter.convert(to:out,error:&conversionError) { _, state in
        if supplied { state.pointee = .endOfStream; return nil }
        supplied=true; state.pointee = .haveData; return buffer
    }
    guard status != .error, out.frameLength > 0 else { throw VoiceError("microphone conversion failed: \(conversionError?.localizedDescription ?? "no output")") }
    return out
}
final class CallbackState: @unchecked Sendable {
    private let lock = NSLock(); private var partial: String?; private var final: String?
    func setPartial(_ s: String) { lock.lock(); partial = s; lock.unlock() }
    func setFinal(_ s: String) { lock.lock(); final = s; lock.unlock() }
    func take() -> (String?, String?) { lock.lock(); defer { lock.unlock() }; let result = (partial, final); partial = nil; final = nil; return result }
}

// Blocking a synthesis callback is intentional backpressure: no PCM is produced while paused.
final class PlaybackGate: @unchecked Sendable {
    private let condition = NSCondition(); private var paused=false; private var cancelled=false; private var permits=6
    func pause() { condition.lock(); paused=true; condition.unlock() }
    func resume() { condition.lock(); paused=false; condition.broadcast(); condition.unlock() }
    func cancel() { condition.lock(); cancelled=true; condition.broadcast(); condition.unlock() }
    func release() { condition.lock(); permits=min(6,permits+1); condition.broadcast(); condition.unlock() }
    func wait() -> Bool { condition.lock(); defer { condition.unlock() }; while (paused || permits == 0) && !cancelled { condition.wait() }; if cancelled { return false }; permits -= 1; return true }
}
final class InputDetector: @unchecked Sendable {
    private let lock=NSLock(); private var muted=false; private var voiced=0; private var active=false; private var lastLevelTime:TimeInterval = 0
    func setMuted(_ value:Bool) { lock.lock(); muted=value; voiced=0; active=false; lock.unlock() }
    func observe(_ buffer:AVAudioPCMBuffer) -> (edge:Bool?,level:Double,report:Bool)? {
        guard let channel=buffer.floatChannelData?[0] else { return nil }
        let n=Int(buffer.frameLength); guard n > 0 else { return nil }
        var sum:Float=0; for i in 0..<n { sum += channel[i]*channel[i] }; let rms=sqrt(sum/Float(n))
        lock.lock(); defer { lock.unlock() }; guard !muted else { return nil }
        voiced = rms > 0.035 ? voiced+1 : 0
        let now=ProcessInfo.processInfo.systemUptime
        let report=now-lastLevelTime >= 0.1
        if report { lastLevelTime=now }
        var edge:Bool?
        if voiced >= 2 && !active { active=true; edge=true }
        if rms < 0.01 && active { active=false; edge=false }
        return (edge,min(1,Double(rms)),report)
    }
}
func inputFinishIsCurrent(expectedSession:String?, expectedEpoch:String?, expectedToken:UUID, expectedRequest:String, session:String?, epoch:String?, token:UUID?, request:String?, cancelled:Bool) -> Bool {
    !cancelled && session == expectedSession && epoch == expectedEpoch && token == expectedToken && request == expectedRequest
}
func captureWatchdogShouldStop(now:TimeInterval, started:TimeInterval, lastTap:TimeInterval, running:Bool, muted:Bool, finishing:Bool, current:Bool) -> Bool {
    current && !muted && !finishing && (!running || now-max(started,lastTap) >= 5)
}
public actor VoiceService {
    private let writer: EventWriter
    private var session: String?; private var epoch: String?
    private var retiredEpochs = Set<String>()
    private var asr: StreamingEouAsrManager?; private var tts: Phonon?
    private var engine: AVAudioEngine?; private var player: AVAudioPlayerNode?; private var testSink=false
    private var inputTask: Task<Void, Never>?; private var speechTask: Task<Void, Never>?; private var playbackGate: PlaybackGate?
    private var finishInputRequest:String?; private var finishInputToken:UUID?
    private var muteDrain: Task<Void, Never>?; private var muteDrainID: UUID?; private var unmuteRequest: String?; private var unmuteID: UUID?
    private var pendingBuffers=0; private var generationFinished=false; private var speechPaused=false; private var samplesPlayed:UInt64=0; private var lastReportedSamples:UInt64=0
    private let inputQueue = InputQueue(); private let callbackState = CallbackState(); private let detector = InputDetector()
    private var lockFD: Int32 = -1
    private var muted = false; private var currentGeneration: UInt64?; private var lastGeneration: UInt64 = 0; private var knownGenerations = Set<UInt64>()
    private var utteranceId = UUID().uuidString.lowercased(); private var revision: UInt64 = 0
    private var voicedFrames = 0; private var activity = false
    private var routeObserver: NSObjectProtocol?
    private var activeRoute: AudioRouteSnapshot?
    public init(writer: EventWriter) { self.writer = writer; AppLogger.minimumLevel = .warning; AppLogger.mirrorsToConsole = false }
    private func emit(_ type: String, _ fields: [String: Any] = [:]) {
        var event: [String: Any] = ["type": type, "sessionId": session ?? "", "epoch": epoch ?? ""]
        fields.forEach { event[$0.key] = $0.value }; writer.send(event)
    }
    private func error(_ command: Command, _ code: String, _ message: String) {
        writer.send(["type":"error", "sessionId":command.sessionId, "epoch":command.epoch, "requestId":command.requestId, "code":code, "message":message])
    }
    public func handle(_ c: Command) async {
        guard UUID(uuidString:c.sessionId) != nil, UUID(uuidString:c.epoch) != nil, !c.requestId.isEmpty else { error(c,"invalidIdentity","sessionId and epoch must be UUID strings and requestId is required"); return }
        if c.type == "load" {
            traceVoice("load-command")
            guard session == nil, muteDrain == nil, !retiredEpochs.contains(c.epoch) else { error(c,"busy","previous session or recognizer cleanup is still active; retired epochs cannot be reused"); return }
            session = c.sessionId; epoch = c.epoch
            do {
                guard let p = c.modelsPath else { throw VoiceError("modelsPath is required") }
                let root = URL(fileURLWithPath:p, isDirectory:true)
                let start = Date()
                if asr == nil {
                    // Keep the GPU free for the desktop UI; measured paced
                    // duplex runs showed lower ASR tail latency on CPU + ANE.
                    let configuration=MLModelConfiguration(); configuration.computeUnits = .cpuAndNeuralEngine
                    let manager = StreamingEouAsrManager(configuration:configuration,chunkSize:.ms320)
                    traceVoice("asr-load-begin")
                    try await manager.loadModels(from:root.appendingPathComponent("parakeet-eou-320"))
                    traceVoice("asr-load-end")
                    let state = callbackState
                    await manager.setPartialCallback { state.setPartial($0) }
                    await manager.setEouCallback { state.setFinal($0) }
                    // Initialize the recognizer's inference path before capture starts.
                    // Silence contains no user audio; reset discards all warm-up state.
                    let warmFormat=AVAudioFormat(standardFormatWithSampleRate:16000,channels:1)!
                    let warm=AVAudioPCMBuffer(pcmFormat:warmFormat,frameCapacity:16000)!
                    warm.frameLength=16000; warm.floatChannelData![0].initialize(repeating:0,count:16000)
                    traceVoice("asr-warm-begin")
                    _ = try await manager.process(audioBuffer:warm)
                    await manager.reset(); _ = state.take()
                    traceVoice("asr-warm-end")
                    asr = manager
                }
                if tts == nil {
                    let dir = root.appendingPathComponent("phonon-31m-en-v0.6.0/phonon-31m-en")
                    traceVoice("tts-load-begin")
                    tts = try await Phonon.load(models:dir, language:.english)
                    traceVoice("tts-load-end")
                }
                traceVoice("load-complete")
                emit("ready",["requestId":c.requestId,"status":"loaded","loadMs":Int(Date().timeIntervalSince(start)*1000)])
                traceVoice("load-ready-written")
            } catch { self.error(c,"loadFailed",String(describing:error)); retiredEpochs.insert(c.epoch); session=nil; epoch=nil }
            return
        }
        guard session == c.sessionId, epoch == c.epoch else { error(c,"staleSession","session or epoch is not active"); return }
        switch c.type {
        case "start":
            traceVoice("capture-start-command")
            do {
                if c.testOnlyNoAudio == true {
                    guard ProcessInfo.processInfo.environment["MONITTER_VOICE_TEST_SINK"] == "1", engine == nil, !testSink, asr != nil, tts != nil else { throw VoiceError("test sink unavailable") }
                    testSink=true; emit("ready",["requestId":c.requestId,"status":"started","voiceProcessing":false,"testOnlyNoAudio":true])
                } else { try await start(); traceVoice("capture-started"); emit("ready",["requestId":c.requestId,"status":"started","voiceProcessing":true]) }
            }
            catch { self.error(c,"startFailed",String(describing:error)) }
        case "stop":
            await stop(); emit("ready",["requestId":c.requestId,"status":"stopped"])
            retiredEpochs.insert(c.epoch); session=nil; epoch=nil
        case "finishInput":
            guard engine != nil, !muted, finishInputRequest == nil, unmuteRequest == nil else { error(c,"invalidInputFinish","input is not capturing or is already finishing"); return }
            let token=UUID(); finishInputToken=token; finishInputRequest=c.requestId
            inputQueue.freeze(); emit("audioLevel",["source":"input","level":0])
            let s=c.sessionId, ep=c.epoch
            Task { [weak self] in try? await Task.sleep(nanoseconds:20_000_000_000); await self?.inputFinishTimedOut(session:s,epoch:ep,token:token) }
        case "mute":
            guard let value=c.muted else { error(c,"invalidCommand","muted is required"); return }
            if value {
                finishInputRequest=nil; finishInputToken=nil
                supersedeUnmute()
                muted=true; detector.setMuted(true); inputQueue.setMuted(true)
                emit("audioLevel",["source":"input","level":0])
                if muteDrain == nil {
                    let oldInput=inputTask; oldInput?.cancel(); inputTask=nil
                    let recognizer=asr, state=callbackState
                    muteDrainID=UUID()
                    muteDrain=Task { traceVoice("mute-drain-await-input"); await oldInput?.value; traceVoice("mute-drain-input-done"); _ = state.take(); await recognizer?.reset(); traceVoice("mute-drain-reset-done") }
                }
                emit("ready",["requestId":c.requestId,"status":"muted"])
            } else {
                guard unmuteRequest == nil, finishInputRequest == nil else { error(c,"busy","input transition already pending"); return }
                if !muted { emit("ready",["requestId":c.requestId,"status":"unmuted"]); return }
                unmuteRequest=c.requestId; let token=UUID(); unmuteID=token
                let drain=muteDrain, s=c.sessionId, ep=c.epoch, request=c.requestId
                Task { [weak self] in await drain?.value; await self?.finishUnmute(session:s,epoch:ep,request:request,token:token) }
                Task { [weak self] in try? await Task.sleep(nanoseconds:20_000_000_000); await self?.unmuteTimedOut(session:s,epoch:ep,request:request,token:token) }
            }
        case "speak":
            guard let generation=c.generation, generation > lastGeneration, let text=c.text, !text.isEmpty, text.utf8.count <= 4096, knownGenerations.count < 4096, (engine != nil || testSink), let tts else { error(c,"invalidSpeech","requires active audio, nonempty text <= 4096 bytes and increasing generation"); return }
            cancelSpeech(status:"cancelled"); lastGeneration=generation; knownGenerations.insert(generation); currentGeneration=generation
            let id=c.sessionId, ep=c.epoch, gate=PlaybackGate(); playbackGate=gate; pendingBuffers=0; generationFinished=false; speechPaused=false; samplesPlayed=0; lastReportedSamples=0
            speechTask = Task { [weak self] in
                guard let service=self else { return }
                do {
                    try await tts.speak(text) { chunk in
                        guard gate.wait() else { return }
                        let done=DispatchSemaphore(value:0)
                        Task { if await service.play(chunk, generation:generation, session:id, epoch:ep) != true { gate.cancel() }; done.signal() }
                        done.wait()
                    }
                    await service.finishSpeech(generation:generation, session:id, epoch:ep)
                } catch { await service.speechFailed(error, generation:generation, session:id, epoch:ep) }
            }
            emit("audioProgress",["requestId":c.requestId,"generation":generation,"status":"started"])
        case "pauseSpeech":
            guard let g=c.generation else { error(c,"staleGeneration","generation is required"); return }
            if currentGeneration != g {
                if knownGenerations.contains(g) { emit("audioProgress",["requestId":c.requestId,"generation":g,"status":"alreadyFinished"]); return }
                error(c,"staleGeneration","generation is unknown"); return
            }
            pauseSpeech(requestId:c.requestId)
        case "resumeSpeech":
            guard let g=c.generation else { error(c,"staleGeneration","generation is required"); return }
            if currentGeneration != g {
                if knownGenerations.contains(g) { emit("audioProgress",["requestId":c.requestId,"generation":g,"status":"alreadyFinished"]); return }
                error(c,"staleGeneration","generation is unknown"); return
            }
            speechPaused=false; player?.play(); playbackGate?.resume()
            emit("audioProgress",["requestId":c.requestId,"generation":g,"status":"resumed"])
        case "cancelSpeech":
            guard let g=c.generation else { error(c,"staleGeneration","generation is required"); return }
            if currentGeneration != g {
                if knownGenerations.contains(g) { emit("audioProgress",["requestId":c.requestId,"generation":g,"status":"alreadyFinished"]); return }
                error(c,"staleGeneration","generation is unknown"); return
            }
            cancelSpeech(status:"cancelled",requestId:c.requestId)
        default: error(c,"invalidCommand","unknown command")
        }
    }
    private func finishUnmute(session s:String, epoch ep:String, request:String, token:UUID) {
        traceVoice("unmute-drain-done")
        guard session == s, epoch == ep, unmuteRequest == request, unmuteID == token else { return }
        unmuteRequest=nil; unmuteID=nil; muteDrain=nil; muteDrainID=nil; muted=false; detector.setMuted(false); inputQueue.setMuted(false)
        utteranceId=UUID().uuidString.lowercased(); revision=0
        if engine != nil { inputTask=Task { [weak self] in await self?.consumeInput(session:s,epoch:ep) } }
        emit("ready",["requestId":request,"status":"unmuted"])
    }
    private func unmuteTimedOut(session s:String, epoch ep:String, request:String, token:UUID) {
        guard session == s, epoch == ep, unmuteRequest == request, unmuteID == token else { return }
        traceVoice("unmute-timeout"); unmuteRequest=nil; unmuteID=nil
        writer.send(["type":"error","sessionId":s,"epoch":ep,"requestId":request,"code":"recognizerBusy","message":"recognizer reset did not finish; input remains muted"])
    }
    private func supersedeUnmute() {
        guard let request=unmuteRequest else { return }
        unmuteRequest=nil; unmuteID=nil
        writer.send(["type":"error","sessionId":session ?? "","epoch":epoch ?? "","requestId":request,"code":"superseded","message":"unmute was superseded; input remains muted"])
    }
    private func start() async throws {
        guard engine == nil, !testSink, asr != nil, tts != nil else { throw VoiceError("models must be loaded and capture must not already be running") }
        traceVoice("permission-check-begin")
        switch AVCaptureDevice.authorizationStatus(for:.audio) {
        case .authorized: traceVoice("permission-authorized")
        case .notDetermined:
            traceVoice("permission-request-begin")
            let granted=await AVCaptureDevice.requestAccess(for:.audio)
            traceVoice(granted ? "permission-request-granted" : "permission-request-denied")
            guard granted else { throw VoiceError("microphone permission denied") }
        default: traceVoice("permission-denied"); throw VoiceError("microphone permission denied")
        }
        traceVoice("capture-lock-begin")
        let path = "/tmp/monitter-native-voice-\(getuid()).lock"
        lockFD = open(path, O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, S_IRUSR | S_IWUSR)
        guard lockFD >= 0 else { throw VoiceError("cannot open microphone lock") }
        guard flock(lockFD, LOCK_EX | LOCK_NB) == 0 else { close(lockFD); lockFD = -1; throw VoiceError("microphone already owned by another voice helper") }
        traceVoice("capture-lock-acquired")
        do {
            traceVoice("engine-create-begin")
            let e=AVAudioEngine(), p=AVAudioPlayerNode()
            traceVoice("engine-create-end")
            let input=e.inputNode, output=e.outputNode
            traceVoice("voice-processing-begin")
            do { try input.setVoiceProcessingEnabled(true); traceVoice("voice-processing-end") }
            catch { throw VoiceError("enable voice processing failed: \(error)") }
            let outputFormat=output.outputFormat(forBus:0)
            guard outputFormat.sampleRate > 0, outputFormat.channelCount > 0 else { throw VoiceError("no usable output route after enabling voice processing") }
            e.attach(p)
            let out=AVAudioFormat(standardFormatWithSampleRate:24000, channels:1)!
            e.connect(p,to:e.mainMixerNode,format:out)
            let format=input.outputFormat(forBus:0), q=inputQueue, detector=detector
            let observedEpoch=epoch, observedSession=session, tapBaseline=q.tapMetrics()
            guard format.sampleRate > 0, format.channelCount > 0 else { throw VoiceError("no microphone route") }
            // VoiceProcessingIO requires matching client-side I/O formats on some
            // macOS routes. The output node converts this to its hardware format;
            // keep the capture format untouched and retain voice processing.
            e.connect(e.mainMixerNode,to:output,format:format)
            input.installTap(onBus:0,bufferSize:4096,format:format) { [weak self] buffer,_ in
                guard let token=q.captureToken() else { return }
                guard let copy=AVAudioPCMBuffer(pcmFormat:buffer.format,frameCapacity:buffer.frameLength) else { return }
                copy.frameLength=buffer.frameLength
                let source=UnsafeMutableAudioBufferListPointer(buffer.mutableAudioBufferList), target=UnsafeMutableAudioBufferListPointer(copy.mutableAudioBufferList)
                for i in 0..<min(source.count,target.count) { if let s=source[i].mData, let d=target[i].mData { memcpy(d,s,Int(source[i].mDataByteSize)); target[i].mDataByteSize=source[i].mDataByteSize } }
                if let observation=detector.observe(copy), let service=self, observation.edge != nil || observation.report { Task { await service.inputObservation(edge:observation.edge, level:observation.level, report:observation.report, session:observedSession, epoch:observedEpoch, token:token) } }
                q.push(copy,token:token)
            }
            traceVoice("engine-start-begin")
            do { try e.start(); traceVoice("engine-start-end") }
            catch { throw VoiceError("start voice-processing engine failed (input \(format.sampleRate) Hz/\(format.channelCount) ch, output \(outputFormat.sampleRate) Hz/\(outputFormat.channelCount) ch): \(error)") }
            p.play(); engine=e; player=p; activeRoute=audioRouteSnapshot(e)
            routeObserver=NotificationCenter.default.addObserver(forName:.AVAudioEngineConfigurationChange,object:e,queue:nil) { [weak self] _ in Task { await self?.routeChanged(session:observedSession,epoch:observedEpoch) } }
            inputTask=Task { [weak self] in await self?.consumeInput(session:observedSession,epoch:observedEpoch) }
            let captureStarted=ProcessInfo.processInfo.systemUptime
            Task { [weak self] in
                try? await Task.sleep(nanoseconds:3_000_000_000)
                await self?.captureCheck(session:observedSession,epoch:observedEpoch,expectedEngine:e,baseline:tapBaseline)
            }
            Task { [weak self] in
                while !Task.isCancelled {
                    try? await Task.sleep(nanoseconds:1_000_000_000)
                    guard await self?.captureWatchdog(session:observedSession,epoch:observedEpoch,expectedEngine:e,started:captureStarted,baseline:tapBaseline) == true else { return }
                }
            }
        } catch { if lockFD >= 0 { flock(lockFD,LOCK_UN); close(lockFD); lockFD = -1 }; throw error }
    }
    private func captureWatchdog(session s:String?, epoch ep:String?, expectedEngine:AVAudioEngine, started:TimeInterval, baseline:(UInt64,UInt64)) -> Bool {
        guard session == s, epoch == ep, engine === expectedEngine else { return false }
        if captureWatchdogShouldStop(now:ProcessInfo.processInfo.systemUptime,started:started,lastTap:inputQueue.lastTapTime(),running:expectedEngine.isRunning,muted:muted,finishing:finishInputRequest != nil,current:true) {
            captureCheck(session:s,epoch:ep,expectedEngine:expectedEngine,baseline:baseline)
            emit("error",["code":"inputUnavailable","message":"Microphone input is unavailable. Check your input device and start voice again."])
            Task { await self.stop() }
            return false
        }
        return true
    }
    private func captureCheck(session s:String?, epoch ep:String?, expectedEngine:AVAudioEngine, baseline:(UInt64,UInt64)) {
        guard session == s, epoch == ep, engine === expectedEngine else { return }
        let (taps,accepted)=inputQueue.tapMetrics()
        traceVoice("capture-check taps=\(taps &- baseline.0) accepted=\(accepted &- baseline.1) running=\(expectedEngine.isRunning) muted=\(muted)")
    }
    private func consumeInput(session s:String?, epoch ep:String?) async {
        defer { traceVoice("input-task-exit") }
        var lastMetrics:TimeInterval=0
        while !Task.isCancelled && session == s && epoch == ep && engine != nil {
            let (buffer,overflow)=inputQueue.take()
            if overflow { if let detail=inputQueue.overflowDetails() { traceVoice(detail.diagnostic) }; emit("error",["code":"inputOverflow","message":"Voice processing fell behind. Voice stopped to avoid losing speech."]); Task { await self.stop() }; return }
            guard let buffer else {
                if let request=finishInputRequest, let token=finishInputToken {
                    do {
                        let final=try await asr?.finish() ?? ""
                        guard inputFinishIsCurrent(expectedSession:s,expectedEpoch:ep,expectedToken:token,expectedRequest:request,session:session,epoch:epoch,token:finishInputToken,request:finishInputRequest,cancelled:Task.isCancelled) else { return }
                        let (_,callbackFinal)=callbackState.take()
                        let text=final.isEmpty ? (callbackFinal ?? "") : final
                        await asr?.reset()
                        guard inputFinishIsCurrent(expectedSession:s,expectedEpoch:ep,expectedToken:token,expectedRequest:request,session:session,epoch:epoch,token:finishInputToken,request:finishInputRequest,cancelled:Task.isCancelled) else { return }
                        if !text.isEmpty { revision += 1; emit("transcript",["utteranceId":utteranceId,"text":text,"isFinal":true,"revision":revision]) }
                        _ = callbackState.take(); finishInputRequest=nil; finishInputToken=nil; muted=true; detector.setMuted(true); inputQueue.setMuted(true); inputTask=nil
                        utteranceId=UUID().uuidString.lowercased(); revision=0
                        emit("ready",["requestId":request,"status":"inputFinished"])
                    } catch {
                        guard finishInputToken == token else { return }
                        finishInputRequest=nil; finishInputToken=nil
                        emit("error",["requestId":request,"code":"inputFinishFailed","message":"Could not finish voice input."]); Task { await self.stop() }
                    }
                    return
                }
                try? await Task.sleep(nanoseconds:10_000_000); continue
            }
            if muted { continue }
            do {
                traceVoice("asr-normalize-begin")
                let normalized=try normalizeInput(buffer)
                traceVoice("asr-process-begin")
                let processStart=ProcessInfo.processInfo.systemUptime
                _ = try await asr?.process(audioBuffer:normalized)
                let processEnd=ProcessInfo.processInfo.systemUptime
                traceVoice("asr-process-end")
                if processEnd-lastMetrics >= 5 {
                    let m=inputQueue.metrics(now:processEnd)
                    traceVoice("input-metrics rate=\(m.rate) frames=\(m.frames) queuedSeconds=\(m.queued) oldestSeconds=\(m.age) count=\(m.count) processSeconds=\(processEnd-processStart)")
                    lastMetrics=processEnd
                }
                guard session == s, epoch == ep, engine != nil, !muted else { continue }
                if finishInputRequest != nil { continue }
                let (partial,final)=callbackState.take()
                if let partial, !partial.isEmpty { revision += 1; emit("transcript",["utteranceId":utteranceId,"text":partial,"isFinal":false,"revision":revision]) }
                if let final, !final.isEmpty {
                    revision += 1; emit("transcript",["utteranceId":utteranceId,"text":final,"isFinal":true,"revision":revision])
                    traceVoice("asr-reset-begin"); await asr?.reset(); traceVoice("asr-reset-end"); utteranceId=UUID().uuidString.lowercased(); revision=0
                }
            } catch {
                if Task.isCancelled { traceVoice("asr-process-cancelled"); return }
                emit("error",["code":"asrFailed","message":String(describing:error)]); Task { await self.stop() }; return
            }
        }
    }
    private func inputFinishTimedOut(session s:String, epoch ep:String, token:UUID) {
        guard session == s, epoch == ep, finishInputToken == token, let request=finishInputRequest else { return }
        finishInputRequest=nil; finishInputToken=nil
        emit("error",["requestId":request,"code":"inputFinishTimeout","message":"Finishing voice input timed out. Voice stopped."])
        Task { await self.stop() }
    }
    private func inputObservation(edge:Bool?, level:Double, report:Bool, session s:String?, epoch ep:String?, token:UInt64) {
        guard session == s, epoch == ep, engine != nil, !muted, finishInputRequest == nil, inputQueue.isCurrent(token) else { return }
        if report { emit("audioLevel",["source":"input","level":level]) }
        if let active=edge {
            if active { pauseSpeech() }
            emit("inputActivity",["active":active,"level":level])
        }
    }
    private func play(_ samples:[Float], generation:UInt64, session:String, epoch:String) -> Bool {
        guard self.session == session, self.epoch == epoch, currentGeneration == generation, !samples.isEmpty, samples.count <= 2400 else { return false }
        if testSink { playbackGate?.release(); return true }
        guard let player else { return false }
        let f=AVAudioFormat(standardFormatWithSampleRate:24000,channels:1)!
        guard let b=AVAudioPCMBuffer(pcmFormat:f,frameCapacity:AVAudioFrameCount(samples.count)), let dst=b.floatChannelData?[0] else { return false }
        b.frameLength=AVAudioFrameCount(samples.count); samples.withUnsafeBufferPointer { if let src=$0.baseAddress { dst.update(from:src,count:samples.count) } }
        pendingBuffers += 1
        let gate=playbackGate
        let count=samples.count
        let level=min(1, sqrt(samples.reduce(0.0) { $0 + Double($1)*Double($1) } / Double(count)))
        player.scheduleBuffer(b, completionCallbackType:.dataPlayedBack) { [weak self] _ in
            gate?.release()
            Task { await self?.bufferPlayed(generation:generation, session:session, epoch:epoch, count:count, level:level) }
        }
        if !speechPaused && !player.isPlaying { player.play() }; return true
    }
    private func finishSpeech(generation:UInt64, session:String, epoch:String) {
        guard self.session == session, self.epoch == epoch, currentGeneration == generation else { return }
        generationFinished=true
        if pendingBuffers == 0 {
            if !testSink && samplesPlayed > lastReportedSamples { emit("audioProgress",["generation":generation,"status":"progress","samplesPlayed":samplesPlayed,"level":0]) }
            currentGeneration=nil; emit("audioProgress",["generation":generation,"status":testSink ? "generated" : "completed"])
        }
    }
    private func bufferPlayed(generation:UInt64, session:String, epoch:String, count:Int, level:Double) {
        guard self.session == session, self.epoch == epoch, currentGeneration == generation else { return }
        pendingBuffers=max(0,pendingBuffers-1)
        samplesPlayed += UInt64(count)
        if samplesPlayed-lastReportedSamples >= 2400 || (generationFinished && pendingBuffers == 0) {
            lastReportedSamples=samplesPlayed
            emit("audioProgress",["generation":generation,"status":"progress","samplesPlayed":samplesPlayed,"level":level])
        }
        if generationFinished && pendingBuffers == 0 { currentGeneration=nil; emit("audioProgress",["generation":generation,"status":"completed"]) }
    }
    private func pauseSpeech(requestId:String? = nil) {
        guard let g=currentGeneration else { return }
        if speechPaused {
            if let requestId { emit("audioProgress",["generation":g,"status":"paused","requestId":requestId]) }
            return
        }
        speechPaused=true; playbackGate?.pause(); player?.pause()
        var fields:[String:Any]=["generation":g,"status":"paused"]; if let requestId { fields["requestId"]=requestId }; emit("audioProgress",fields)
    }
    private func speechFailed(_ error:Error, generation:UInt64, session:String, epoch:String) {
        guard self.session == session, self.epoch == epoch, currentGeneration == generation else { return }
        cancelSpeech(status:"cancelled"); emit("error",["code":"synthesisFailed","message":String(describing:error)])
    }
    private func cancelSpeech(status:String,requestId:String? = nil) {
        guard let g=currentGeneration else { return }
        currentGeneration=nil; playbackGate?.cancel(); playbackGate=nil; speechTask?.cancel(); speechTask=nil; player?.stop(); player?.play(); pendingBuffers=0; generationFinished=false; speechPaused=false
        var fields:[String:Any]=["generation":g,"status":status]; if let requestId { fields["requestId"]=requestId }; emit("audioProgress",fields)
    }
    private func routeChanged(session s:String?, epoch ep:String?) async {
        guard session == s, epoch == ep, let engine else { return }
        let current=audioRouteSnapshot(engine), running=engine.isRunning
        let mustStop=routeNotificationRequiresStop(running:running,expected:activeRoute,current:current)
        traceVoice("route-notification running=\(running) snapshotMatch=\(activeRoute != nil && activeRoute == current) stop=\(mustStop)")
        guard mustStop else { return }
        emit("error",["code":"routeChanged","message":"The audio device changed. Start voice again to continue."])
        await stop()
    }
    public func stop() async {
        finishInputRequest=nil; finishInputToken=nil; supersedeUnmute(); muted=true; detector.setMuted(true); inputQueue.setMuted(true)
        cancelSpeech(status:"cancelled"); let previousInput=inputTask; previousInput?.cancel(); inputTask=nil
        if let observer=routeObserver { NotificationCenter.default.removeObserver(observer); routeObserver=nil }
        engine?.inputNode.removeTap(onBus:0); engine?.stop(); player=nil; engine=nil
        activeRoute=nil
        if muteDrain == nil {
            let recognizer=asr, state=callbackState
            muteDrainID=UUID()
            muteDrain=Task { await previousInput?.value; _ = state.take(); await recognizer?.reset() }
        }
        let drain=muteDrain, drainID=muteDrainID
        Task { [weak self] in await drain?.value; await self?.finishStoppedDrain(drainID) }
        if lockFD >= 0 { flock(lockFD,LOCK_UN); close(lockFD); lockFD = -1 }
        testSink=false; activity=false; voicedFrames=0; lastGeneration=0; knownGenerations.removeAll(); utteranceId=UUID().uuidString.lowercased(); revision=0
    }
    private func finishStoppedDrain(_ id:UUID?) { if engine == nil, let id, muteDrainID == id { muteDrain=nil; muteDrainID=nil; muted=false; detector.setMuted(false); inputQueue.setMuted(false) } }
#if DEBUG
    // Test-only seams; no audio device is started or accessed.
    func testSetActiveInput(session s:String, epoch ep:String, task:Task<Void,Never>) {
        session=s; epoch=ep; engine=AVAudioEngine(); inputTask=task; muted=false
    }
    func testCancelCurrentInput() { inputTask?.cancel(); inputTask=nil; engine=nil }
    func testInstallDrain(_ id:UUID, task:Task<Void,Never>) { muteDrainID=id; muteDrain=task; muted=true }
    func testFinishDrain(_ id:UUID) { finishStoppedDrain(id) }
    func testDrainID() -> UUID? { muteDrainID }
#endif
}
struct VoiceError: Error, CustomStringConvertible { let description:String; init(_ s:String) { description=s } }
