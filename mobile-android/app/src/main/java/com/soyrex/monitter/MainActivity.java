package com.soyrex.monitter;

import android.annotation.SuppressLint;
import android.app.Activity;
import android.Manifest;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.speech.RecognitionListener;
import android.speech.RecognizerIntent;
import android.speech.SpeechRecognizer;
import android.util.Log;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.webkit.ConsoleMessage;
import android.webkit.PermissionRequest;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.FrameLayout;
import android.widget.TextView;

import androidx.webkit.WebViewAssetLoader;
import androidx.webkit.WebViewCompat;
import androidx.webkit.WebViewFeature;

import com.google.android.gms.tasks.OnFailureListener;
import com.google.mlkit.vision.barcode.common.Barcode;
import com.google.mlkit.vision.codescanner.GmsBarcodeScanner;
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions;
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning;

import java.io.IOException;
import java.io.InputStream;
import java.util.Collections;
import java.util.ArrayList;

import org.json.JSONObject;

/** Native container for the shared Monitter mobile and paired-owner SPA. */
public final class MainActivity extends Activity {
    private static final String ASSET_ORIGIN = "https://appassets.androidplatform.net";
    private static final String MOBILE_URL = ASSET_ORIGIN + "/mobile";
    private static final int MAX_QR_LENGTH = 4096;
    private static final int AUDIO_PERMISSION_REQUEST = 7041;
    private static final int WEB_AUDIO_PERMISSION_REQUEST = 7042;
    private static final long SPEECH_MAX_DURATION_MS = 90_000L;
    private WebView webView;
    private TextView errorView;
    private GmsBarcodeScanner qrScanner;
    private boolean scanInProgress;
    private SpeechRecognizer speechRecognizer;
    private String speechSessionId;
    private String lastSpeechPartial;
    private boolean speechStopRequested;
    private String pendingSpeechSessionId;
    private String pendingSpeechLocale;
    private PermissionRequest pendingWebAudioPermission;
    private final Handler mainHandler = new Handler(Looper.getMainLooper());
    private final Runnable speechTimeout = () -> stopSpeech(false);
    private final Runnable speechStopFallback = () -> {
        if (speechSessionId != null && lastSpeechPartial != null && !lastSpeechPartial.trim().isEmpty()) {
            dispatchSpeech(speechSessionId, "final", null, null, lastSpeechPartial);
            completeSpeech(speechSessionId);
        } else finishSpeech("stopped");
    };

    @SuppressLint("SetJavaScriptEnabled")
    @Override
    public void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().setSoftInputMode(android.view.WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);

        WebView.setWebContentsDebuggingEnabled(BuildConfig.DEBUG);
        WebViewAssetLoader assetLoader = new WebViewAssetLoader.Builder()
                .addPathHandler("/", new WebViewAssetLoader.AssetsPathHandler(this))
                .build();

        webView = new WebView(this);
        webView.setBackgroundColor(Color.rgb(18, 17, 15));
        // Android 15 draws edge-to-edge by default. Keep the mobile header and
        // composer clear of the status, gesture-navigation, and IME insets.
        webView.setOnApplyWindowInsetsListener((view, insets) -> {
            view.setPadding(0, insets.getSystemWindowInsetTop(), 0,
                    insets.getSystemWindowInsetBottom());
            return insets;
        });
        WebSettings settings = webView.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setDatabaseEnabled(false);
        settings.setAllowFileAccess(false);
        settings.setAllowContentAccess(false);
        settings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        settings.setMediaPlaybackRequiresUserGesture(true);
        settings.setSupportMultipleWindows(false);

        webView.setWebViewClient(new LocalContentClient(assetLoader));
        webView.setWebChromeClient(new WebChromeClient() {
            @Override
            public void onPermissionRequest(PermissionRequest request) {
                runOnUiThread(() -> handleWebPermissionRequest(request));
            }

            @Override
            public void onPermissionRequestCanceled(PermissionRequest request) {
                if (pendingWebAudioPermission == request) pendingWebAudioPermission = null;
            }

            @Override
            public boolean onConsoleMessage(ConsoleMessage message) {
                if (message.messageLevel() == ConsoleMessage.MessageLevel.ERROR) {
                    Log.e("MonitterWeb", message.message() + " (" + message.sourceId()
                            + ":" + message.lineNumber() + ")");
                }
                return false;
            }
        });

        FrameLayout root = new FrameLayout(this);
        root.addView(webView, new ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        errorView = new TextView(this);
        errorView.setBackgroundColor(Color.rgb(18, 17, 15));
        errorView.setTextColor(Color.rgb(245, 241, 232));
        errorView.setGravity(Gravity.CENTER);
        errorView.setPadding(48, 48, 48, 48);
        errorView.setTextSize(16);
        errorView.setVisibility(View.GONE);
        root.addView(errorView, new ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        setContentView(root);
        installQrBridge();
        installSpeechBridge();
        if (savedInstanceState == null) {
            webView.loadUrl(MOBILE_URL);
        } else if (webView.restoreState(savedInstanceState) == null) {
            webView.loadUrl(MOBILE_URL);
        }
    }

    @Override
    protected void onSaveInstanceState(Bundle outState) {
        webView.saveState(outState);
        super.onSaveInstanceState(outState);
    }

    @Override
    public void onBackPressed() {
        // The SPA owns in-app navigation. If there is no mobile destination to
        // close, background the task rather than destroying its WebView/session.
        webView.evaluateJavascript("Boolean(window.monitterHandleBack && window.monitterHandleBack())",
                result -> {
                    if (!"true".equals(result)) {
                        moveTaskToBack(true);
                    }
                });
    }

    @Override
    protected void onResume() {
        super.onResume();
        if (webView != null) {
            dispatchToPage("monitter:resume", "");
        }
    }

    @Override
    protected void onPause() {
        cancelSpeech(true);
        cancelPendingWebAudioPermission();
        super.onPause();
    }

    @Override
    protected void onDestroy() {
        cancelSpeech(false);
        cancelPendingWebAudioPermission();
        destroySpeechRecognizer();
        super.onDestroy();
    }

    private void showError(String message) {
        Log.e("MonitterWeb", message);
        errorView.setText("Monitter could not load.\n\n" + message);
        errorView.setVisibility(View.VISIBLE);
    }

    private void installQrBridge() {
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
            Log.w("MonitterWeb", "WebView does not support the QR message bridge.");
            return;
        }
        WebViewCompat.addWebMessageListener(webView, "monitterAndroid",
                Collections.singleton(ASSET_ORIGIN),
                (view, message, sourceOrigin, isMainFrame, replyProxy) -> {
                    if (!isMainFrame || !ASSET_ORIGIN.equals(sourceOrigin.toString())
                            || !"scanQR".equals(message.getData())) {
                        Log.w("MonitterWeb", "Rejected an untrusted QR bridge message.");
                        return;
                    }
                    view.post(this::startQrScan);
                });
    }

    private void startQrScan() {
        if (scanInProgress) {
            return;
        }
        scanInProgress = true;
        try {
            if (qrScanner == null) {
                GmsBarcodeScannerOptions options = new GmsBarcodeScannerOptions.Builder()
                        .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
                        .enableAutoZoom()
                        .build();
                qrScanner = GmsBarcodeScanning.getClient(this, options);
            }
            qrScanner.startScan()
                    .addOnSuccessListener(barcode -> {
                        scanInProgress = false;
                        String rawValue = barcode.getRawValue();
                        if (rawValue == null || rawValue.isEmpty() || rawValue.length() > MAX_QR_LENGTH) {
                            dispatchToPage("monitter:qr-error", "The scanned code is not a valid Monitter invitation.");
                            return;
                        }
                        dispatchToPage("monitter:qr", rawValue);
                    })
                    .addOnCanceledListener(() -> scanInProgress = false)
                    .addOnFailureListener((OnFailureListener) error -> {
                        scanInProgress = false;
                        Log.e("MonitterWeb", "QR scanner failed", error);
                        dispatchToPage("monitter:qr-error",
                                "The QR scanner is unavailable. Check Google Play services and try again.");
                    });
        } catch (RuntimeException error) {
            scanInProgress = false;
            Log.e("MonitterWeb", "Unable to start QR scanner", error);
            dispatchToPage("monitter:qr-error",
                    "The QR scanner is unavailable. Check Google Play services and try again.");
        }
    }

    private void installSpeechBridge() {
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
            Log.w("MonitterWeb", "WebView does not support the speech message bridge.");
            return;
        }
        WebViewCompat.addWebMessageListener(webView, "monitterSpeechAndroid",
                Collections.singleton(ASSET_ORIGIN),
                (view, message, sourceOrigin, isMainFrame, replyProxy) -> {
                    if (!isMainFrame || !ASSET_ORIGIN.equals(sourceOrigin.toString())) {
                        Log.w("MonitterWeb", "Rejected an untrusted speech bridge message.");
                        return;
                    }
                    view.post(() -> handleSpeechMessage(message.getData()));
                });
    }

    private void handleSpeechMessage(String rawMessage) {
        try {
            JSONObject message = new JSONObject(rawMessage);
            String action = message.optString("action", "");
            String sessionId = message.optString("sessionId", "");
            if (sessionId.isEmpty() || sessionId.length() > 128) {
                return;
            }
            if ("status".equals(action)) {
                dispatchSpeech(sessionId, "availability", isOnDeviceSpeechAvailable(), null, null);
            } else if ("start".equals(action)) {
                String locale = message.optString("locale", "");
                if (locale.length() > 64) locale = "";
                beginSpeech(sessionId, locale);
            } else if ("stop".equals(action)) {
                if (sessionId.equals(speechSessionId)) stopSpeech(false);
            } else if ("cancel".equals(action)) {
                if (sessionId.equals(speechSessionId) || sessionId.equals(pendingSpeechSessionId)) {
                    cancelSpeech(true);
                }
            }
        } catch (Exception error) {
            Log.w("MonitterWeb", "Rejected invalid speech bridge payload.", error);
        }
    }

    private boolean isOnDeviceSpeechAvailable() {
        return Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
                && SpeechRecognizer.isOnDeviceRecognitionAvailable(this);
    }

    private void beginSpeech(String sessionId, String locale) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
            dispatchSpeech(sessionId, "error", null,
                    "On-device speech recognition requires Android 12 or newer.", null);
            return;
        }
        if (!isOnDeviceSpeechAvailable()) {
            dispatchSpeech(sessionId, "error", null,
                    "On-device speech recognition is unavailable on this device.", null);
            return;
        }
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            pendingSpeechSessionId = sessionId;
            pendingSpeechLocale = locale;
            requestPermissions(new String[]{Manifest.permission.RECORD_AUDIO}, AUDIO_PERMISSION_REQUEST);
            return;
        }
        startOnDeviceSpeech(sessionId, locale);
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode == AUDIO_PERMISSION_REQUEST) {
            String sessionId = pendingSpeechSessionId;
            String locale = pendingSpeechLocale;
            pendingSpeechSessionId = null;
            pendingSpeechLocale = null;
            if (sessionId == null) return;
            if (grantResults.length > 0 && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
                startOnDeviceSpeech(sessionId, locale == null ? "" : locale);
            } else {
                dispatchSpeech(sessionId, "error", null,
                        "Microphone access was denied. Allow microphone access and try again.", null);
            }
        } else if (requestCode == WEB_AUDIO_PERMISSION_REQUEST) {
            PermissionRequest request = pendingWebAudioPermission;
            pendingWebAudioPermission = null;
            if (request == null) return;
            if (grantResults.length > 0 && grantResults[0] == PackageManager.PERMISSION_GRANTED
                    && isOwnerRelayOrigin(Uri.parse(request.getOrigin().toString()))
                    && isCurrentOwnerRelayPage()) {
                try {
                    request.grant(new String[]{PermissionRequest.RESOURCE_AUDIO_CAPTURE});
                } catch (RuntimeException ignored) { }
            } else {
                try { request.deny(); } catch (RuntimeException ignored) { }
            }
        }
    }

    private void handleWebPermissionRequest(PermissionRequest request) {
        String[] resources = request.getResources();
        if (resources == null || resources.length != 1
                || !PermissionRequest.RESOURCE_AUDIO_CAPTURE.equals(resources[0])
                || !isOwnerRelayOrigin(Uri.parse(request.getOrigin().toString()))
                || !isCurrentOwnerRelayPage()) {
            try { request.deny(); } catch (RuntimeException ignored) { }
            return;
        }
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
            try { request.grant(resources); } catch (RuntimeException ignored) { }
            return;
        }
        if (pendingWebAudioPermission != null) {
            try { request.deny(); } catch (RuntimeException ignored) { }
            return;
        }
        pendingWebAudioPermission = request;
        requestPermissions(new String[]{Manifest.permission.RECORD_AUDIO}, WEB_AUDIO_PERMISSION_REQUEST);
    }

    private void cancelPendingWebAudioPermission() {
        PermissionRequest request = pendingWebAudioPermission;
        pendingWebAudioPermission = null;
        if (request != null) {
            try { request.deny(); } catch (RuntimeException ignored) { }
        }
    }

    private void startOnDeviceSpeech(String sessionId, String locale) {
        cancelSpeech(true);
        try {
            speechRecognizer = SpeechRecognizer.createOnDeviceSpeechRecognizer(this);
            speechSessionId = sessionId;
            lastSpeechPartial = null;
            speechStopRequested = false;
            final String callbackSessionId = sessionId;
            speechRecognizer.setRecognitionListener(new RecognitionListener() {
                @Override public void onReadyForSpeech(Bundle params) { }
                @Override public void onBeginningOfSpeech() { }
                @Override public void onRmsChanged(float rmsdB) { }
                @Override public void onBufferReceived(byte[] buffer) { }
                @Override public void onEndOfSpeech() { }
                @Override public void onEvent(int eventType, Bundle params) { }
                @Override public void onPartialResults(Bundle results) {
                    if (callbackSessionId.equals(speechSessionId)) {
                        emitSpeechResult(callbackSessionId, results, "partial");
                    }
                }
                @Override public void onResults(Bundle results) {
                    if (!callbackSessionId.equals(speechSessionId)) return;
                    if (emitSpeechResult(callbackSessionId, results, "final")) {
                        completeSpeech(callbackSessionId);
                    } else if (lastSpeechPartial != null && !lastSpeechPartial.trim().isEmpty()) {
                        dispatchSpeech(callbackSessionId, "final", null, null, lastSpeechPartial);
                        completeSpeech(callbackSessionId);
                    } else {
                        finishSpeech(callbackSessionId, "stopped");
                    }
                }
                @Override public void onError(int error) {
                    if (!callbackSessionId.equals(speechSessionId)) return;
                    if (speechStopRequested && lastSpeechPartial != null && !lastSpeechPartial.trim().isEmpty()) {
                        dispatchSpeech(callbackSessionId, "final", null, null, lastSpeechPartial);
                        completeSpeech(callbackSessionId);
                    } else {
                        finishSpeech(callbackSessionId, "error", speechErrorMessage(error));
                    }
                }
            });
            Intent intent = new Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH);
            intent.putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL,
                    RecognizerIntent.LANGUAGE_MODEL_FREE_FORM);
            if (!locale.isEmpty()) intent.putExtra(RecognizerIntent.EXTRA_LANGUAGE, locale);
            intent.putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true);
            intent.putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 1);
            speechRecognizer.startListening(intent);
            mainHandler.removeCallbacks(speechTimeout);
            mainHandler.postDelayed(speechTimeout, SPEECH_MAX_DURATION_MS);
            dispatchSpeech(sessionId, "started", null, null, null);
        } catch (RuntimeException error) {
            Log.e("MonitterWeb", "Unable to start on-device speech recognition", error);
            finishSpeech("error", "Could not start speech recognition. Try again.");
        }
    }

    private boolean emitSpeechResult(String sessionId, Bundle results, String type) {
        if (!sessionId.equals(speechSessionId) || results == null) return false;
        ArrayList<String> matches = results.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION);
        if (matches != null && !matches.isEmpty()) {
            if ("partial".equals(type)) lastSpeechPartial = matches.get(0);
            dispatchSpeech(sessionId, type, null, null, matches.get(0));
            return true;
        }
        return false;
    }

    private void stopSpeech(boolean canceled) {
        mainHandler.removeCallbacks(speechTimeout);
        if (speechRecognizer == null || speechSessionId == null) return;
        try {
            if (canceled) speechRecognizer.cancel();
            else { speechStopRequested = true; speechRecognizer.stopListening(); }
        } catch (RuntimeException ignored) { }
        if (canceled) finishSpeech("stopped");
        else {
            mainHandler.removeCallbacks(speechStopFallback);
            mainHandler.postDelayed(speechStopFallback, 8_000L);
        }
    }

    private void cancelSpeech(boolean notify) {
        String pendingSessionId = pendingSpeechSessionId;
        pendingSpeechSessionId = null;
        pendingSpeechLocale = null;
        if (speechRecognizer != null) {
            try { speechRecognizer.cancel(); } catch (RuntimeException ignored) { }
        }
        if (speechSessionId != null && notify) finishSpeech("stopped");
        else {
            speechSessionId = null;
            lastSpeechPartial = null;
            speechStopRequested = false;
            mainHandler.removeCallbacks(speechTimeout);
            mainHandler.removeCallbacks(speechStopFallback);
            destroySpeechRecognizer();
            if (pendingSessionId != null && notify) {
                dispatchSpeech(pendingSessionId, "stopped", null, null, null);
            }
        }
    }

    private void finishSpeech(String type) { finishSpeech(type, null); }

    private void finishSpeech(String type, String error) {
        finishSpeech(speechSessionId, type, error);
    }

    private void finishSpeech(String expectedSessionId, String type, String error) {
        if (expectedSessionId == null || !expectedSessionId.equals(speechSessionId)) return;
        String sessionId = expectedSessionId;
        speechSessionId = null;
        lastSpeechPartial = null;
        speechStopRequested = false;
        mainHandler.removeCallbacks(speechTimeout);
        mainHandler.removeCallbacks(speechStopFallback);
        destroySpeechRecognizer();
        if (sessionId != null) dispatchSpeech(sessionId, type, null, error, null);
    }

    private void completeSpeech(String expectedSessionId) {
        if (expectedSessionId == null || !expectedSessionId.equals(speechSessionId)) return;
        speechSessionId = null;
        lastSpeechPartial = null;
        speechStopRequested = false;
        mainHandler.removeCallbacks(speechTimeout);
        mainHandler.removeCallbacks(speechStopFallback);
        destroySpeechRecognizer();
    }

    private void destroySpeechRecognizer() {
        if (speechRecognizer != null) {
            try { speechRecognizer.destroy(); } catch (RuntimeException ignored) { }
            speechRecognizer = null;
        }
    }

    private String speechErrorMessage(int error) {
        if (error == SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS) {
            return "Microphone access is unavailable. Check app permissions and try again.";
        }
        if (error == SpeechRecognizer.ERROR_NO_MATCH || error == SpeechRecognizer.ERROR_SPEECH_TIMEOUT) {
            return "No speech was recognized. Try again.";
        }
        return "Speech recognition failed. Check the on-device language pack and try again.";
    }

    private void dispatchSpeech(String sessionId, String type, Boolean available,
                                String error, String text) {
        if (webView == null) return;
        JSONObject detail = new JSONObject();
        try {
            detail.put("sessionId", sessionId);
            detail.put("type", type);
            if (available != null) detail.put("available", available);
            if (error != null) detail.put("error", error);
            if (text != null) detail.put("text", text);
        } catch (org.json.JSONException ignored) { return; }
        String script = "window.dispatchEvent(new CustomEvent('monitter:native-speech', {detail: "
                + "JSON.parse(" + JSONObject.quote(detail.toString()) + ")}));";
        webView.evaluateJavascript(script, null);
    }

    private void dispatchToPage(String eventName, String detail) {
        String script = "window.dispatchEvent(new CustomEvent(" + JSONObject.quote(eventName)
                + ", { detail: " + JSONObject.quote(detail) + " }));";
        webView.evaluateJavascript(script, null);
    }

    private final class LocalContentClient extends WebViewClient {
        private final WebViewAssetLoader assetLoader;

        LocalContentClient(WebViewAssetLoader assetLoader) {
            this.assetLoader = assetLoader;
        }

        @Override
        public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
            Uri url = request.getUrl();
            if (isAppEntryRoute(url)) {
                return mobileIndex();
            }
            return assetLoader.shouldInterceptRequest(url);
        }

        @Override
        public void onPageStarted(WebView view, String url, android.graphics.Bitmap favicon) {
            cancelPendingWebAudioPermission();
            super.onPageStarted(view, url, favicon);
        }

        @Override
        public void onPageFinished(WebView view, String url) {
            if (isBundledEntryUrl(Uri.parse(url))) {
                errorView.setVisibility(View.GONE);
            }
        }

        @Override
        public void onReceivedError(WebView view, WebResourceRequest request,
                                    android.webkit.WebResourceError error) {
            if (request.isForMainFrame()) {
                showError("Error " + error.getErrorCode() + ": " + error.getDescription());
            }
        }

        @Override
        public void onReceivedHttpError(WebView view, WebResourceRequest request,
                                        WebResourceResponse response) {
            if (request.isForMainFrame()) {
                showError("HTTP " + response.getStatusCode() + " while loading the bundled mobile app.");
            }
        }

        @Override
        public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
            // The WebView is intentionally restricted to its bundled HTTPS origin.
            return !isBundledAssetUrl(request.getUrl());
        }

        private boolean isAppEntryRoute(Uri url) {
            if (!isBundledAssetUrl(url)) return false;
            String path = url.getPath();
            return path == null || path.isEmpty() || "/".equals(path)
                    || "/mobile".equals(path) || path.startsWith("/mobile/");
        }

        private WebResourceResponse mobileIndex() {
            try {
                InputStream content = getAssets().open("index.html");
                return new WebResourceResponse("text/html", "UTF-8", content);
            } catch (IOException error) {
                return new WebResourceResponse("text/plain", "UTF-8", null);
            }
        }
    }

    private boolean isBundledEntryUrl(Uri url) {
        if (!isBundledAssetUrl(url)) return false;
        String path = url.getPath();
        return path == null || path.isEmpty() || "/".equals(path)
                || "/mobile".equals(path) || path.startsWith("/mobile/");
    }

    private boolean isOwnerRelayPage(Uri url) {
        if (!isBundledAssetUrl(url)) return false;
        String path = url.getPath();
        return path == null || path.isEmpty() || "/".equals(path);
    }

    private boolean isCurrentOwnerRelayPage() {
        return webView != null && webView.getUrl() != null
                && isOwnerRelayPage(Uri.parse(webView.getUrl()));
    }

    private boolean isOwnerRelayOrigin(Uri origin) {
        return isBundledAssetUrl(origin)
                && (origin.getPath() == null || origin.getPath().isEmpty() || "/".equals(origin.getPath()));
    }

    private boolean isBundledAssetUrl(Uri url) {
        return url.getUserInfo() == null
                && "https".equals(url.getScheme())
                && "appassets.androidplatform.net".equals(url.getHost())
                && (url.getPort() == -1 || url.getPort() == 443);
    }
}
