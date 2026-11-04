// swift-tools-version: 6.0
import PackageDescription
import Foundation
let plist = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("Info.plist").path
let package = Package(
    name: "MonitterNativeVoice", platforms: [.macOS(.v15)],
    products: [.executable(name: "monitter-native-voice", targets: ["VoiceHelper"])],
    dependencies: [
        .package(url: "https://github.com/FluidInference/FluidAudio.git", revision: "ee78635d23d52379117554ba72358b65a0612447"),
        .package(url: "https://github.com/gradium-ai/xn-ptts.git", revision: "95bf24d1ca5e94f0f62abe588b1ccb6f093bcc9c"),
    ],
    targets: [
        .target(name: "VoiceCore", dependencies: [.product(name: "FluidAudio", package: "FluidAudio"), .product(name: "ptts", package: "xn-ptts")]),
        .executableTarget(name: "VoiceHelper", dependencies: ["VoiceCore"], linkerSettings: [.unsafeFlags(["-Xlinker", "-sectcreate", "-Xlinker", "__TEXT", "-Xlinker", "__info_plist", "-Xlinker", plist])]),
        .testTarget(name: "VoiceCoreTests", dependencies: ["VoiceCore"]),
    ], swiftLanguageModes: [.v5]
)
