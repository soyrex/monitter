import XCTest
import AVFoundation
import CoreML
import FluidAudio
import PhononTTS
@testable import VoiceCore

final class VoiceTests: XCTestCase {
    func testRouteNotificationsFailClosedUnlessRunningSnapshotMatches() {
        let route=AudioRouteSnapshot(inputDevice:1,outputDevice:2,inputRate:16000,inputChannels:1,outputRate:16000,outputChannels:1)
        let changedDevice=AudioRouteSnapshot(inputDevice:1,outputDevice:3,inputRate:16000,inputChannels:1,outputRate:16000,outputChannels:1)
        let changedFormat=AudioRouteSnapshot(inputDevice:1,outputDevice:2,inputRate:16000,inputChannels:1,outputRate:44100,outputChannels:2)
        XCTAssertFalse(routeNotificationRequiresStop(running:true,expected:route,current:route))
        XCTAssertTrue(routeNotificationRequiresStop(running:false,expected:route,current:route))
        XCTAssertTrue(routeNotificationRequiresStop(running:true,expected:route,current:changedDevice))
        XCTAssertTrue(routeNotificationRequiresStop(running:true,expected:route,current:changedFormat))
        XCTAssertTrue(routeNotificationRequiresStop(running:true,expected:route,current:nil))
    }
    func testRedundantUnmuteKeepsOriginalInputConsumer() async throws {
        let service=VoiceService(writer:EventWriter()), session=UUID().uuidString, epoch=UUID().uuidString
        let ended=expectation(description:"original input consumer is still tracked")
        let original=Task { try? await Task.sleep(nanoseconds:60_000_000_000); ended.fulfill() }
        await service.testSetActiveInput(session:session,epoch:epoch,task:original)
        let data=try JSONSerialization.data(withJSONObject:["type":"mute","sessionId":session,"epoch":epoch,"requestId":"duplicate-unmute","muted":false])
        await service.handle(try JSONDecoder().decode(Command.self,from:data))
        await service.testCancelCurrentInput()
        await fulfillment(of:[ended],timeout:2)
    }
    func testStaleStoppedCleanupCannotClearNewDrain() async {
        let service=VoiceService(writer:EventWriter()), old=UUID(), current=UUID()
        let task=Task { _ = try? await Task.sleep(nanoseconds:60_000_000_000) }
        await service.testInstallDrain(current,task:task)
        await service.testFinishDrain(old)
        let afterStale=await service.testDrainID()
        XCTAssertEqual(afterStale,current)
        await service.testFinishDrain(current)
        let afterCurrent=await service.testDrainID()
        XCTAssertNil(afterCurrent)
        task.cancel()
    }
    func testSyntheticRecognizerCancellation() async throws {
        guard let root=ProcessInfo.processInfo.environment["VOICE_MODELS"], let fixture=ProcessInfo.processInfo.environment["VOICE_FIXTURE"] else { throw XCTSkip("Set VOICE_MODELS and VOICE_FIXTURE") }
        let asr=StreamingEouAsrManager(chunkSize:.ms320)
        try await asr.loadModels(from:URL(fileURLWithPath:root).appendingPathComponent("parakeet-eou-320"))
        let file=try AVAudioFile(forReading:URL(fileURLWithPath:fixture))
        let buffer=AVAudioPCMBuffer(pcmFormat:file.processingFormat,frameCapacity:AVAudioFrameCount(file.processingFormat.sampleRate))!
        try file.read(into:buffer)
        let normalized=try normalizeInput(buffer)
        for label in ["first", "second"] {
            let started=Date()
            _ = try await asr.process(audioBuffer:normalized)
            print("VOICE synthetic \(label) inference seconds=\(Date().timeIntervalSince(started))")
            await asr.reset()
        }
        let finished=expectation(description:"recognizer returns after cancellation")
        let task=Task.detached { defer { finished.fulfill() }; _ = try? await asr.process(audioBuffer:normalized) }
        try await Task.sleep(nanoseconds:200_000_000)
        let start=Date(); task.cancel()
        await fulfillment(of:[finished],timeout:15)
        print("VOICE synthetic cancellation seconds=\(Date().timeIntervalSince(start))")
        await asr.reset()
    }
    func testInputNormalizationAndMuteGate() throws {
        let f=AVAudioFormat(standardFormatWithSampleRate:48000,channels:2)!
        let b=AVAudioPCMBuffer(pcmFormat:f,frameCapacity:4800)!
        b.frameLength=4800
        for channel in 0..<2 { for i in 0..<4800 { b.floatChannelData![channel][i]=0.1 } }
        let normalized=try normalizeInput(b)
        XCTAssertEqual(normalized.format.sampleRate,16000)
        XCTAssertEqual(normalized.format.channelCount,1)
        XCTAssertGreaterThan(normalized.frameLength,1500)
        XCTAssertLessThanOrEqual(normalized.frameLength,1600)
        let q=InputQueue(); q.push(b); q.setMuted(true); q.push(b)
        XCTAssertNil(q.take().0); XCTAssertFalse(q.take().1)
        q.setMuted(false); q.push(b); XCTAssertNotNil(q.take().0)
    }
    func testFinishRejectsLateCompletionAfterTimeoutOrNewSession() {
        let token=UUID()
        func valid(_ session:String? = "s", _ epoch:String? = "e", _ current:UUID? = nil, _ request:String? = "r", _ cancelled:Bool = false) -> Bool {
            inputFinishIsCurrent(expectedSession:"s",expectedEpoch:"e",expectedToken:token,expectedRequest:"r",session:session,epoch:epoch,token:current,request:request,cancelled:cancelled)
        }
        XCTAssertTrue(valid("s","e",token))
        XCTAssertFalse(valid("s","e",nil,nil)) // timeout or mute retires token
        XCTAssertFalse(valid("s","e",UUID())) // later finish
        XCTAssertFalse(valid("new","e",token))
        XCTAssertFalse(valid("s","new",token))
        XCTAssertFalse(valid("s","e",token,"r",true))
    }
    func testCaptureWatchdogDetectsMissingBuffersAndRespectsState() {
        func stop(now:Double=15, started:Double=10, tap:Double=0, running:Bool=true, muted:Bool=false, finishing:Bool=false, current:Bool=true) -> Bool {
            captureWatchdogShouldStop(now:now,started:started,lastTap:tap,running:running,muted:muted,finishing:finishing,current:current)
        }
        XCTAssertFalse(stop(now:14.99)); XCTAssertTrue(stop())
        XCTAssertFalse(stop(now:15,tap:14)) // quiet buffers still arrive
        XCTAssertTrue(stop(now:20,tap:15))
        XCTAssertFalse(stop(muted:true)); XCTAssertFalse(stop(finishing:true))
        XCTAssertFalse(stop(current:false)); XCTAssertTrue(stop(running:false))
    }
    func testFreezeDrainsAcceptedInputAndRejectsLateCapture() {
        let q=InputQueue(), f=AVAudioFormat(standardFormatWithSampleRate:16000,channels:1)!
        let b=AVAudioPCMBuffer(pcmFormat:f,frameCapacity:1600)!; b.frameLength=1600
        let token=q.captureToken()!
        q.push(b,token:token); q.freeze()
        XCTAssertNil(q.captureToken())
        q.push(b,token:token)
        XCTAssertTrue(q.take().0 === b); XCTAssertNil(q.take().0)
        q.setMuted(true); q.setMuted(false)
        XCTAssertFalse(q.isCurrent(token)); XCTAssertNotNil(q.captureToken())
    }
    func testInputQueueSurvivesTransientStallAndPreservesOrder() {
        let seconds=[0.064,0.128,0.192,0.064,0.128,0.192,0.064,0.128,0.128,0.128,0.128]
        for rate in [16000.0, 48000.0] {
            let q=InputQueue(), f=AVAudioFormat(standardFormatWithSampleRate:rate,channels:1)!
            var input:[AVAudioPCMBuffer]=[]; var elapsed=0.0
            for duration in seconds {
                let frames=AVAudioFrameCount((rate*duration).rounded())
                let b=AVAudioPCMBuffer(pcmFormat:f,frameCapacity:frames)!; b.frameLength=frames
                input.append(b); q.push(b,now:elapsed); elapsed += duration
            }
            XCTAssertGreaterThan(elapsed,1.3)
            for original in input { let (next,overflow)=q.take(now:elapsed); XCTAssertFalse(overflow); XCTAssertTrue(next === original) }
            XCTAssertNil(q.take(now:elapsed).0)
        }
    }
    func testInputQueueBoundsDurationAgeAndMuteAtDifferentRates() {
        let q=InputQueue()
        for rate in [16000.0, 48000.0] {
            let f=AVAudioFormat(standardFormatWithSampleRate:rate,channels:1)!
            let b=AVAudioPCMBuffer(pcmFormat:f,frameCapacity:AVAudioFrameCount(rate))!; b.frameLength=b.frameCapacity
            for i in 0..<4 { q.push(b,now:Double(i)*0.1) }
            q.push(b,now:0.4)
            XCTAssertTrue(q.take(now:0.4).1, "audio duration must be bounded independently of sample rate")
            let detail=q.overflowDetails()!
            XCTAssertEqual(detail.reason,"duration"); XCTAssertEqual(detail.queuedSeconds,4,accuracy:0.0001)
            XCTAssertEqual(detail.count,4); XCTAssertEqual(detail.frames,UInt32(rate)); XCTAssertEqual(detail.rate,rate)
            q.clear()
            q.push(b,now:0); XCTAssertTrue(q.take(now:4).1, "old backlog must fail closed"); XCTAssertEqual(q.overflowDetails()?.reason,"age")
            q.setMuted(true); q.push(b,now:5); XCTAssertNil(q.take(now:5).0); XCTAssertFalse(q.take(now:5).1)
            q.setMuted(false); q.push(b,now:6); XCTAssertTrue(q.take(now:6).0 === b)
            let oldToken=q.captureToken()!
            q.setMuted(true); XCTAssertNil(q.captureToken()); q.setMuted(false)
            q.push(b,now:6,token:oldToken); XCTAssertNil(q.take(now:6).0)
            XCTAssertFalse(q.isCurrent(oldToken))
            q.clear()
        }
    }
    func testInputQueueBoundsManySmallBuffers() {
        let q=InputQueue(), f=AVAudioFormat(standardFormatWithSampleRate:48000,channels:1)!
        let b=AVAudioPCMBuffer(pcmFormat:f,frameCapacity:1)!; b.frameLength=1
        for _ in 0..<1025 { q.push(b,now:0) }
        XCTAssertTrue(q.take(now:0).1); XCTAssertEqual(q.overflowDetails()?.reason,"count")
    }
    func testInputQueueBoundsAllocatedBytes() {
        let q=InputQueue(), f=AVAudioFormat(standardFormatWithSampleRate:48000,channels:2)!
        let b=AVAudioPCMBuffer(pcmFormat:f,frameCapacity:1_100_000)!; b.frameLength=48_000
        q.push(b,now:0); XCTAssertTrue(q.take(now:0).1); XCTAssertEqual(q.overflowDetails()?.reason,"bytes")
    }
    func testGateCancellationReleasesPausedProducer() async {
        let gate=PlaybackGate(); gate.pause()
        let t=Task.detached { gate.wait() }
        try? await Task.sleep(nanoseconds:30_000_000)
        gate.cancel()
        let accepted=await t.value
        XCTAssertFalse(accepted)
    }
    func testGateLimitsQueuedBuffers() async {
        let gate=PlaybackGate()
        for _ in 0..<6 { XCTAssertTrue(gate.wait()) }
        let t=Task.detached { gate.wait() }
        try? await Task.sleep(nanoseconds:30_000_000)
        gate.release()
        let accepted=await t.value
        XCTAssertTrue(accepted)
    }
    func testBenchmarkCPUANELoad() async throws {
        guard let root=ProcessInfo.processInfo.environment["VOICE_MODELS"], ProcessInfo.processInfo.environment["VOICE_BENCH_CPU_ANE"] == "1" else { throw XCTSkip("Set VOICE_MODELS and VOICE_BENCH_CPU_ANE=1") }
        let config=MLModelConfiguration(); config.computeUnits = .cpuAndNeuralEngine
        let asr=StreamingEouAsrManager(configuration:config,chunkSize:.ms320), start=ProcessInfo.processInfo.systemUptime
        try await asr.loadModels(from:URL(fileURLWithPath:root).appendingPathComponent("parakeet-eou-320"))
        print("VOICE cpu-ane-load seconds=\(ProcessInfo.processInfo.systemUptime-start)")
    }
    func testPacedDuplexSyntheticASR() async throws {
        guard let root=ProcessInfo.processInfo.environment["VOICE_MODELS"], let fixture=ProcessInfo.processInfo.environment["VOICE_FIXTURE"], ProcessInfo.processInfo.environment["VOICE_PACED_DUPLEX"] == "1" else { throw XCTSkip("Set VOICE_MODELS, VOICE_FIXTURE and VOICE_PACED_DUPLEX=1") }
        let configuration=MLModelConfiguration()
        if ProcessInfo.processInfo.environment["VOICE_BENCH_CPU_ANE"] == "1" { configuration.computeUnits = .cpuAndNeuralEngine }
        let asr=StreamingEouAsrManager(configuration:configuration,chunkSize:.ms320)
        try await asr.loadModels(from:URL(fileURLWithPath:root).appendingPathComponent("parakeet-eou-320"))
        let callbacks=CallbackState(); await asr.setPartialCallback { callbacks.setPartial($0) }; await asr.setEouCallback { callbacks.setFinal($0) }
        let tts=try await Phonon.load(models:URL(fileURLWithPath:root).appendingPathComponent("phonon-31m-en-v0.6.0/phonon-31m-en"),language:.english)
        let q=InputQueue(), clockStart=ProcessInfo.processInfo.systemUptime
        let producer=Task {
            let file=try AVAudioFile(forReading:URL(fileURLWithPath:fixture)), format=file.processingFormat
            var sent:Double=0, silenceChunks=0
            while sent < 60 {
                if file.framePosition >= file.length && silenceChunks == 0 { file.framePosition=0; silenceChunks=13 }
                let b=AVAudioPCMBuffer(pcmFormat:format,frameCapacity:AVAudioFrameCount(format.sampleRate*0.32))!
                if silenceChunks > 0 {
                    b.frameLength=b.frameCapacity
                    for channel in 0..<Int(format.channelCount) { b.floatChannelData![channel].initialize(repeating:0,count:Int(b.frameLength)) }
                    silenceChunks -= 1
                } else { try file.read(into:b) }
                guard b.frameLength > 0 else { throw VoiceError("empty synthetic fixture") }
                q.push(b)
                let duration=Double(b.frameLength)/format.sampleRate; sent += duration
                try await Task.sleep(nanoseconds:UInt64(duration*1_000_000_000))
            }
        }
        let ttsTask=Task {
            while ProcessInfo.processInfo.systemUptime-clockStart < 60 && !Task.isCancelled {
                _ = try await tts.synthesize("This is a local synthetic playback test, repeated while recognition processes speech input.")
                try await Task.sleep(nanoseconds:1_000_000_000)
            }
        }
        var maxAge=0.0, maxQueued=0.0, maxReset=0.0, maxProcess=0.0, calls=0, finals=0, overflow=false
        while ProcessInfo.processInfo.systemUptime-clockStart < 65 {
            let (buffer,failed)=q.take()
            if failed { overflow=true; break }
            let metrics=q.metrics(); maxAge=max(maxAge,metrics.age); maxQueued=max(maxQueued,metrics.queued)
            guard let buffer else { try await Task.sleep(nanoseconds:10_000_000); continue }
            let processStart=ProcessInfo.processInfo.systemUptime
            _ = try await asr.process(audioBuffer:try normalizeInput(buffer)); maxProcess=max(maxProcess,ProcessInfo.processInfo.systemUptime-processStart); calls += 1
            let (_,final)=callbacks.take()
            if let final, !final.isEmpty { finals += 1; let resetStart=ProcessInfo.processInfo.systemUptime; await asr.reset(); maxReset=max(maxReset,ProcessInfo.processInfo.systemUptime-resetStart) }
            if producer.isCancelled { break }
        }
        producer.cancel(); ttsTask.cancel(); _ = try? await producer.value; _ = try? await ttsTask.value
        print("VOICE paced-duplex seconds=\(ProcessInfo.processInfo.systemUptime-clockStart) calls=\(calls) finals=\(finals) maxProcess=\(maxProcess) maxReset=\(maxReset) maxAge=\(maxAge) maxQueued=\(maxQueued) overflow=\(overflow)")
        XCTAssertFalse(overflow, "Synthetic capture queue overflowed")
        XCTAssertGreaterThan(calls,0)
    }
    func testSustainedSyntheticASRThroughput() async throws {
        guard let root=ProcessInfo.processInfo.environment["VOICE_MODELS"], let fixture=ProcessInfo.processInfo.environment["VOICE_FIXTURE"] else { throw XCTSkip("Set VOICE_MODELS and VOICE_FIXTURE") }
        let asr=StreamingEouAsrManager(chunkSize:.ms320)
        try await asr.loadModels(from:URL(fileURLWithPath:root).appendingPathComponent("parakeet-eou-320"))
        let ttsTask:Task<Void,Error>?
        if ProcessInfo.processInfo.environment["VOICE_BENCH_TTS"] == "1" {
            let tts=try await Phonon.load(models:URL(fileURLWithPath:root).appendingPathComponent("phonon-31m-en-v0.6.0/phonon-31m-en"),language:.english)
            ttsTask=Task {
                for _ in 0..<3 {
                    _ = try await tts.synthesize("This is a local playback test. We are measuring concurrent speech synthesis and recognition during a conversation.")
                }
            }
        } else { ttsTask=nil }
        let file=try AVAudioFile(forReading:URL(fileURLWithPath:fixture)), format=file.processingFormat
        let audioSeconds=30.0, target=Int64(audioSeconds*format.sampleRate)
        var frames:Int64=0, durations:[Double]=[]
        let began=ProcessInfo.processInfo.systemUptime
        while frames < target {
            if file.framePosition >= file.length { file.framePosition=0 }
            let buffer=AVAudioPCMBuffer(pcmFormat:format,frameCapacity:AVAudioFrameCount(format.sampleRate*0.32))!
            try file.read(into:buffer)
            guard buffer.frameLength > 0 else { throw VoiceError("empty synthetic fixture") }
            let start=ProcessInfo.processInfo.systemUptime
            _ = try await asr.process(audioBuffer:buffer)
            durations.append(ProcessInfo.processInfo.systemUptime-start)
            frames += Int64(buffer.frameLength)
        }
        let elapsed=ProcessInfo.processInfo.systemUptime-began
        let sorted=durations.sorted()
        print("VOICE throughput audioSeconds=\(Double(frames)/format.sampleRate) elapsedSeconds=\(elapsed) calls=\(durations.count) p90Seconds=\(sorted[Int(Double(sorted.count-1)*0.9)]) maxSeconds=\(sorted.last ?? 0)")
        try await ttsTask?.value
        XCTAssertGreaterThan(frames,0)
    }
    func testModelsOnSyntheticSpeech() async throws {
        guard let root=ProcessInfo.processInfo.environment["VOICE_MODELS"], let fixture=ProcessInfo.processInfo.environment["VOICE_FIXTURE"] else { throw XCTSkip("Set VOICE_MODELS and VOICE_FIXTURE to run local model measurement") }
        let base=URL(fileURLWithPath:root), start=Date()
        let asr=StreamingEouAsrManager(chunkSize:.ms320)
        try await asr.loadModels(from:base.appendingPathComponent("parakeet-eou-320"))
        print("VOICE ASR load seconds=\(Date().timeIntervalSince(start))")
        let ttsStart=Date(), tts=try await Phonon.load(models:base.appendingPathComponent("phonon-31m-en-v0.6.0/phonon-31m-en"),language:.english)
        print("VOICE TTS load seconds=\(Date().timeIntervalSince(ttsStart)) voices=\(tts.voices.count)")
        let synthStart=Date(), samples=try await tts.synthesize("Hello there. This is a local voice test.")
        print("VOICE TTS synth seconds=\(Date().timeIntervalSince(synthStart)) audio seconds=\(Double(samples.count)/24000)")
        XCTAssertGreaterThan(samples.count,24000)
        let callbacks=CallbackState(); await asr.setPartialCallback { callbacks.setPartial($0) }; await asr.setEouCallback { callbacks.setFinal($0) }
        let file=try AVAudioFile(forReading:URL(fileURLWithPath:fixture)), format=file.processingFormat
        let duration=Double(file.length)/format.sampleRate
        let began=Date(); var chunks=0; var partials=0; var finalBeforeFinish:String?
        while file.framePosition < file.length {
            let b=AVAudioPCMBuffer(pcmFormat:format,frameCapacity:AVAudioFrameCount(format.sampleRate*0.32))!
            try file.read(into:b); if b.frameLength == 0 { break }
            _ = try await asr.process(audioBuffer:b); chunks += 1
            let (partial,final)=callbacks.take()
            if let partial, !partial.isEmpty { partials += 1 }
            if let final, !final.isEmpty { finalBeforeFinish=final }
        }
        // Exercise the live EOU callback before finish(), with bounded silence.
        let silenceFormat=AVAudioFormat(standardFormatWithSampleRate:16000,channels:1)!
        for _ in 0..<13 {
            let silence=AVAudioPCMBuffer(pcmFormat:silenceFormat,frameCapacity:5120)!
            silence.frameLength=5120; silence.floatChannelData![0].initialize(repeating:0,count:5120)
            _ = try await asr.process(audioBuffer:silence)
            let (_,final)=callbacks.take(); if let final, !final.isEmpty { finalBeforeFinish=final; break }
        }
        print("VOICE EOU before-finish=\(finalBeforeFinish ?? "<none>")")
        XCTAssertNotNil(finalBeforeFinish, "EOU callback must fire without finish()")
        let text=try await asr.finish()
        print("VOICE ASR chunks=\(chunks) partials-before-finish=\(partials) audio seconds=\(duration) elapsed seconds=\(Date().timeIntervalSince(began)) text=\(text)")
        XCTAssertGreaterThan(chunks,2)
        XCTAssertGreaterThan(partials,0)
        XCTAssertFalse(text.isEmpty)
    }
}
