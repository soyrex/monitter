#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include "voice-preview-config.h"

static void fail(const char *message) { perror(message); _exit(126); }
static void redirect_log(const char *path, int fd) {
    int out = open(path, O_WRONLY | O_APPEND | O_CREAT | O_NOFOLLOW, 0600);
    if (out < 0 || dup2(out, fd) < 0) fail("preview log");
    if (out != fd) close(out);
}

int main(int argc, char **argv) {
    (void)argc;
    umask(077);
    redirect_log(PREVIEW_STDOUT, STDOUT_FILENO);
    redirect_log(PREVIEW_STDERR, STDERR_FILENO);
    if (unsetenv("OPENAI_API_KEY") || unsetenv("CODEX_API_KEY") ||
        setenv("MONITTER_LOCAL_VOICE", "1", 1) ||
        setenv("MONITTER_NATIVE_VOICE_HELPER", PREVIEW_HELPER, 1) ||
        setenv("MONITTER_VOICE_MODELS", PREVIEW_MODELS, 1) ||
        setenv("MONITTER_GPUI_PROFILE_DIR", PREVIEW_PROFILE, 1) ||
        (strcmp(PREVIEW_DIAGNOSTICS, "1") == 0 ? setenv("MONITTER_VOICE_DIAGNOSTICS", "1", 1) : unsetenv("MONITTER_VOICE_DIAGNOSTICS")) ||
        chdir(PREVIEW_ROOT)) fail("preview environment");
    int marker = open(PREVIEW_PID, O_WRONLY | O_CREAT | O_TRUNC | O_NOFOLLOW, 0600);
    if (marker < 0) fail("preview pid marker");
    char pid[32]; int length = snprintf(pid, sizeof(pid), "%ld\n", (long)getpid());
    if (length <= 0 || write(marker, pid, (size_t)length) != length || close(marker)) fail("preview pid marker");
    argv[0] = PREVIEW_EXECUTABLE;
    execv(PREVIEW_EXECUTABLE, argv);
    fail("preview exec");
}
