// macOS Speech.framework adapter. The caller is a Rust blocking worker; all
// authorization and recognition work is scheduled on the app's main queue.
// Audio is a validated, private temporary WAV owned and removed by Rust.
#import <Foundation/Foundation.h>
#import <Speech/Speech.h>
#import <dispatch/dispatch.h>
#include <stdlib.h>
#include <string.h>

@interface MonitterSpeechOutcome : NSObject
@property (nonatomic, strong) NSLock *lock;
@property (nonatomic, strong) dispatch_semaphore_t ready;
@property (nonatomic, strong) SFSpeechRecognitionTask *task;
@property (nonatomic, copy) NSString *text;
@property (nonatomic, copy) NSString *error;
@property (nonatomic, assign) BOOL finished;
- (void)finishWithText:(NSString *)text error:(NSString *)error;
@end

@implementation MonitterSpeechOutcome
- (instancetype)init {
    self = [super init];
    if (self) {
        _lock = [[NSLock alloc] init];
        _ready = dispatch_semaphore_create(0);
    }
    return self;
}
- (void)finishWithText:(NSString *)text error:(NSString *)error {
    [self.lock lock];
    if (!self.finished) {
        self.finished = YES;
        self.text = text;
        self.error = error;
        dispatch_semaphore_signal(self.ready);
    }
    [self.lock unlock];
}
@end

static char *monitterSpeechCopy(NSString *message) {
    return strdup((message ?: @"Apple on-device transcription failed.").UTF8String);
}

// Both returned strings and error strings are malloc-owned. The Rust caller
// frees them with monitter_apple_speech_free.
char *monitter_apple_speech_transcribe_file(const char *inputPath, char **errorOut) {
    if (errorOut) *errorOut = NULL;
    @autoreleasepool {
        if (!inputPath || !errorOut) return NULL;
        NSString *path = [NSString stringWithUTF8String:inputPath];
        if (!path || ![[NSFileManager defaultManager] fileExistsAtPath:path]) {
            *errorOut = monitterSpeechCopy(@"Voice recording is unavailable for Apple transcription.");
            return NULL;
        }
        SFSpeechRecognizer *recognizer = [[SFSpeechRecognizer alloc] initWithLocale:[NSLocale currentLocale]];
        if (!recognizer || !recognizer.isAvailable || !recognizer.supportsOnDeviceRecognition) {
            *errorOut = monitterSpeechCopy(@"Apple on-device speech is unavailable for this Mac's current language. Choose Whisper in Settings.");
            return NULL;
        }

        SFSpeechRecognizerAuthorizationStatus authorization = [SFSpeechRecognizer authorizationStatus];
        if (authorization == SFSpeechRecognizerAuthorizationStatusNotDetermined) {
            dispatch_semaphore_t permissionReady = dispatch_semaphore_create(0);
            __block SFSpeechRecognizerAuthorizationStatus granted = SFSpeechRecognizerAuthorizationStatusNotDetermined;
            dispatch_async(dispatch_get_main_queue(), ^{
                [SFSpeechRecognizer requestAuthorization:^(SFSpeechRecognizerAuthorizationStatus status) {
                    granted = status;
                    dispatch_semaphore_signal(permissionReady);
                }];
            });
            if (dispatch_semaphore_wait(permissionReady, dispatch_time(DISPATCH_TIME_NOW, 60 * NSEC_PER_SEC)) != 0) {
                *errorOut = monitterSpeechCopy(@"Speech recognition permission timed out. Try again after responding to macOS.");
                return NULL;
            }
            authorization = granted;
        }
        if (authorization != SFSpeechRecognizerAuthorizationStatusAuthorized) {
            *errorOut = monitterSpeechCopy(@"Speech recognition access is denied. Allow Monitter in macOS Privacy & Security settings or choose Whisper.");
            return NULL;
        }

        SFSpeechURLRecognitionRequest *request = [[SFSpeechURLRecognitionRequest alloc] initWithURL:[NSURL fileURLWithPath:path]];
        request.requiresOnDeviceRecognition = YES;
        request.shouldReportPartialResults = NO;
        MonitterSpeechOutcome *outcome = [[MonitterSpeechOutcome alloc] init];
        dispatch_async(dispatch_get_main_queue(), ^{
            outcome.task = [recognizer recognitionTaskWithRequest:request resultHandler:^(SFSpeechRecognitionResult *result, NSError *error) {
                if (result.isFinal) {
                    [outcome finishWithText:result.bestTranscription.formattedString error:nil];
                } else if (error) {
                    // Do not expose system error descriptions, which can contain file paths.
                    [outcome finishWithText:nil error:@"Apple speech could not transcribe this recording. Choose Whisper in Settings or try again."];
                }
            }];
            if (!outcome.task) {
                [outcome finishWithText:nil error:@"Apple speech could not start recognition."];
            }
        });

        if (dispatch_semaphore_wait(outcome.ready, dispatch_time(DISPATCH_TIME_NOW, 180 * NSEC_PER_SEC)) != 0) {
            dispatch_async(dispatch_get_main_queue(), ^{ [outcome.task cancel]; });
            *errorOut = monitterSpeechCopy(@"Apple speech transcription timed out.");
            return NULL;
        }
        [outcome.lock lock];
        NSString *text = [outcome.text copy];
        NSString *error = [outcome.error copy];
        [outcome.lock unlock];
        if (error || text.length == 0) {
            *errorOut = monitterSpeechCopy(error ?: @"Apple speech returned no transcript.");
            return NULL;
        }
        return monitterSpeechCopy(text);
    }
}

void monitter_apple_speech_free(char *value) {
    free(value);
}
