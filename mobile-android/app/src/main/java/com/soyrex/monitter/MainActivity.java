package com.soyrex.monitter;

import android.annotation.SuppressLint;
import android.app.Activity;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.util.Log;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.webkit.ConsoleMessage;
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

import org.json.JSONObject;

/** Native container for the shared Monitter /mobile SPA. */
public final class MainActivity extends Activity {
    private static final String ASSET_ORIGIN = "https://appassets.androidplatform.net";
    private static final String MOBILE_URL = ASSET_ORIGIN + "/mobile";
    private static final int MAX_QR_LENGTH = 4096;
    private WebView webView;
    private TextView errorView;
    private GmsBarcodeScanner qrScanner;
    private boolean scanInProgress;

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
            if (isMobileRoute(url)) {
                return mobileIndex();
            }
            return assetLoader.shouldInterceptRequest(url);
        }

        @Override
        public void onPageFinished(WebView view, String url) {
            if (MOBILE_URL.equals(url)) {
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
            return !ASSET_ORIGIN.equals(request.getUrl().getScheme() + "://" + request.getUrl().getAuthority());
        }

        private boolean isMobileRoute(Uri url) {
            if (!"https".equals(url.getScheme()) || !"appassets.androidplatform.net".equals(url.getHost())) {
                return false;
            }
            String path = url.getPath();
            return "/mobile".equals(path) || (path != null && path.startsWith("/mobile/"));
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
}
