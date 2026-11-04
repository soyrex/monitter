import Foundation
import Darwin
import VoiceCore

// A blocking read on the async main executor can starve the native run loop
// while an otherwise idle voice session is waiting for its next command.
private let inputReader = DispatchQueue(label:"monitter.voice.stdin",qos:.userInitiated)
private func readInput() async -> Data? {
    await withCheckedContinuation { continuation in
        inputReader.async {
            var buffer=[UInt8](repeating:0,count:4096)
            var count:Int
            repeat { count=read(STDIN_FILENO,&buffer,buffer.count) } while count < 0 && errno == EINTR
            continuation.resume(returning:count > 0 ? Data(buffer.prefix(count)) : nil)
        }
    }
}

@main struct Main {
    static func main() async {
        let activity=ProcessInfo.processInfo.beginActivity(options:.userInitiatedAllowingIdleSystemSleep,reason:"Local voice conversation")
        defer { ProcessInfo.processInfo.endActivity(activity) }
        let writer=EventWriter(), service=VoiceService(writer:writer)
        var line=Data(), discarding=false
        while let buffer=await readInput() {
            for byte in buffer {
                if byte == 10 {
                    if discarding { writer.send(["type":"error","code":"invalidJSON","message":"line exceeds 16384 bytes"]) }
                    else {
                        do { let command=try JSONDecoder().decode(Command.self,from:line); await service.handle(command) }
                        catch { writer.send(["type":"error","code":"invalidJSON","message":String(describing:error)]) }
                    }
                    line.removeAll(keepingCapacity:true); discarding=false
                } else if !discarding {
                    if line.count == 16384 { line.removeAll(keepingCapacity:true); discarding=true }
                    else { line.append(byte) }
                }
            }
        }
        await service.stop()
    }
}
