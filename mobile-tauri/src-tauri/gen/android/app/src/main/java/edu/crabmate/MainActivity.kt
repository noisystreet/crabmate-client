package edu.crabmate

import android.os.Build
import android.os.Bundle
import android.view.View
import android.view.WindowManager
import android.view.autofill.AutofillManager
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AlertDialog
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import kotlin.math.roundToInt

class MainActivity : TauriActivity() {
  /** 与 Tauri Android 默认资产源一致（`useHttpsScheme=false` → http）。 */
  private var connectHomeUrl: String = "http://tauri.localhost/"
  private var appWebView: WebView? = null
  private var exitConfirmDialog: AlertDialog? = null
  /**
   * 系统返回键：弹确认框（退出 / 回连接页），不走 WebView.goBack。
   *
   * Tauri AppPlugin 在构造时（Rust `register_android_plugin` → PluginManager.load）
   * 会注册默认回调并对 canGoBack 执行 goBack；若退回无 `?manual=1` 的连接页会自动登录。
   * AppPlugin 往往晚于 [onWebViewCreate]，故除创建时钉回调外，还在 [onResume] 与短暂延迟窗口内重钉。
   */
  private var backPressedCallback: OnBackPressedCallback? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    // 不要 enableEdgeToEdge()：Android WebView 通常不提供 CSS safe-area-inset-*，
    // 铺满状态栏后会与远程 Web 顶栏按钮重叠、无法点击。
    WindowCompat.setDecorFitsSystemWindows(window, true)
    // 键盘弹出时缩小窗口，避免盖住底部 composer（仅靠 visualViewport CSS 在系统 WebView 上常失效）。
    window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
    super.onCreate(savedInstanceState)
    // Tauri / Activity 基类可能在 super 里改回 edge-to-edge，再强制一次。
    WindowCompat.setDecorFitsSystemWindows(window, true)
    window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)

    installBackPressedHandler()
  }

  override fun onResume() {
    super.onResume()
    // AppPlugin 可能在首次 resume 前后才 load；每次回前台再盖过其 goBack 回调。
    installBackPressedHandler()
    trimHistoryIfRemote()
  }

  /** 注册（或重新注册）返回键回调，确保优先于 Tauri AppPlugin 的默认 goBack。 */
  private fun installBackPressedHandler() {
    backPressedCallback?.remove()
    val callback =
      object : OnBackPressedCallback(true) {
        override fun handleOnBackPressed() {
          val url = appWebView?.url
          if (isAppOrigin(url)) {
            // 连接页：确认后退出
            showExitConfirmDialog(fromRemote = false)
          } else {
            // 远程主界面：可退出 App，或回到连接页换服务器
            showExitConfirmDialog(fromRemote = true)
          }
        }
      }
    backPressedCallback = callback
    onBackPressedDispatcher.addCallback(this, callback)
  }

  /**
   * 已在远程 serve 时清掉退回连接页的历史。
   * 若 AppPlugin 仍抢到 Back：canGoBack==false 时会转交 activity.onBackPressed，落到我们的确认框。
   */
  private fun trimHistoryIfRemote() {
    val view = appWebView ?: return
    if (!isAppOrigin(view.url) && view.canGoBack()) {
      view.clearHistory()
    }
  }

  /**
   * WebView 就绪后短窗口内多次重钉回调，覆盖 Rust 侧晚到的 AppPlugin 构造。
   * 同时在已导航到远程时 trim history。
   */
  private fun scheduleBackHandlerDominance(webView: WebView) {
    installBackPressedHandler()
    // 含较晚一档：覆盖慢速自动登录 navigate，以及 Rust 侧晚到的 AppPlugin.load。
    for (delayMs in longArrayOf(0L, 100L, 500L, 2000L, 5000L)) {
      webView.postDelayed(
        {
          installBackPressedHandler()
          trimHistoryIfRemote()
        },
        delayMs,
      )
    }
  }

  /**
   * 系统返回键确认框。
   * @param fromRemote 远程主界面时额外提供「返回连接页」。
   */
  private fun showExitConfirmDialog(fromRemote: Boolean) {
    if (exitConfirmDialog?.isShowing == true) {
      return
    }
    val builder =
      AlertDialog.Builder(this)
        .setTitle(R.string.exit_confirm_title)
        .setMessage(
          if (fromRemote) {
            R.string.exit_confirm_message_remote
          } else {
            R.string.exit_confirm_message
          },
        )
        .setNegativeButton(R.string.exit_confirm_cancel, null)
        .setPositiveButton(R.string.exit_confirm_ok) { _, _ -> finishAffinity() }
        .setOnDismissListener { exitConfirmDialog = null }
    if (fromRemote) {
      builder.setNeutralButton(R.string.exit_confirm_to_connect) { _, _ -> loadConnectPage() }
    }
    exitConfirmDialog = builder.show()
  }

  override fun onStart() {
    super.onStart()
    WindowCompat.setDecorFitsSystemWindows(window, true)
    window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    appWebView = webView
    WindowCompat.setDecorFitsSystemWindows(window, true)
    window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
    // 与前端 `--bg` 对齐：远程 HTML/WASM 未就绪前避免系统默认纯黑空页。
    webView.setBackgroundColor(android.graphics.Color.parseColor("#0A0D12"))
    // 允许系统 Autofill / 密码管理器填充连接页的 URL+Bearer
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      webView.importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_YES
    }
    webView.addJavascriptInterface(MobileBridge(), "CrabMateMobile")
    // 页面加载后注入上下安全区 / IME CSS 变量
    webView.post { injectSafeInsetsCss(webView, null) }
    ViewCompat.setOnApplyWindowInsetsListener(webView) { v, insets ->
      injectSafeInsetsCss(v as? WebView ?: webView, insets)
      // 勿 CONSUMED：让 WebView/Chromium 仍能收到 insets（visualViewport / safe-area）。
      insets
    }
    webView.post { rememberConnectHomeIfAppOrigin(webView.url) }
    scheduleBackHandlerDominance(webView)
  }

  /**
   * 写入远程 Web 的安全区与软键盘 inset。
   * `--cm-ime-inset`：IME 相对导航栏多出的高度（targetSdk 35+ 上 adjustResize 常不缩小窗口时的兜底）。
   */
  private fun injectSafeInsetsCss(webView: WebView, insets: WindowInsetsCompat?) {
    val topPx = statusBarInsetCssPx()
    val bottomPx = navBarInsetCssPx()
    val imePx = imeInsetCssPx(insets)
    val js =
      "(function(){try{var r=document.documentElement;" +
        "r.style.setProperty('--cm-safe-top','${topPx}px');" +
        "r.style.setProperty('--cm-safe-bottom','${bottomPx}px');" +
        "r.style.setProperty('--cm-ime-inset','${imePx}px');" +
        "r.setAttribute('data-cm-mobile-shell','');}catch(e){}})();"
    webView.evaluateJavascript(js, null)
  }

  /** 软键盘相对导航栏的额外高度（CSS px）；无键盘时为 0。 */
  private fun imeInsetCssPx(insets: WindowInsetsCompat?): Int {
    val density = resources.displayMetrics.density.coerceAtLeast(0.5f)
    val src = insets ?: ViewCompat.getRootWindowInsets(window.decorView) ?: return 0
    val ime = src.getInsets(WindowInsetsCompat.Type.ime()).bottom
    val nav = src.getInsets(WindowInsetsCompat.Type.navigationBars()).bottom
    // ime 通常含导航区；与 `--cm-safe-bottom` 分工，避免导航高度算两次。
    return ((ime - nav).coerceAtLeast(0) / density).roundToInt()
  }

  private fun rememberConnectHomeIfAppOrigin(url: String?) {
    if (url.isNullOrBlank() || !isAppOrigin(url)) {
      return
    }
    connectHomeUrl = stripFragmentAndQuery(url).ifBlank { "http://tauri.localhost/" }
  }

  private fun loadConnectPage() {
    val view = appWebView ?: return
    rememberConnectHomeIfAppOrigin(view.url)
    val base = connectHomeUrl.ifBlank { "http://tauri.localhost/" }
    // ?manual=1：跳过连接页冷启动自动登录，便于更换服务器
    val sep = if (base.contains('?')) '&' else '?'
    view.loadUrl("$base${sep}manual=1")
    // 页面提交后再清后退栈（慢网多试一次）；避免 goBack 退回远程或误走无 manual 的历史项。
    for (delayMs in longArrayOf(400L, 1200L)) {
      view.postDelayed(
        {
          if (isAppOrigin(view.url)) {
            view.clearHistory()
          }
        },
        delayMs,
      )
    }
  }

  private fun autofillManager(): AutofillManager? {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
      return null
    }
    return getSystemService(AutofillManager::class.java)
  }

  /** 顶栏安全区（CSS px）：状态栏/刘海高度 + 少量触控余量，供 Web `--cm-safe-top`。 */
  private fun statusBarInsetCssPx(): Int {
    val density = resources.displayMetrics.density.coerceAtLeast(0.5f)
    var topPx = 0
    val types = WindowInsetsCompat.Type.statusBars() or WindowInsetsCompat.Type.displayCutout()
    ViewCompat.getRootWindowInsets(window.decorView)?.let { insets ->
      topPx = insets.getInsetsIgnoringVisibility(types).top
    }
    if (topPx <= 0) {
      val id = resources.getIdentifier("status_bar_height", "dimen", "android")
      if (id > 0) {
        topPx = resources.getDimensionPixelSize(id)
      }
    }
    val css = (topPx / density).roundToInt()
    // 已 setDecorFitsSystemWindows(true) 时 WebView 多数已避开状态栏；仅保留小余量与保底，
    // 避免再叠一层过大 padding（旧：+28 且至少 52）。
    return (css + 4).coerceAtLeast(24)
  }

  /** 系统导航栏/手势条高度（CSS px），供底栏状态条避开。 */
  private fun navBarInsetCssPx(): Int {
    val density = resources.displayMetrics.density.coerceAtLeast(0.5f)
    var bottomPx = 0
    ViewCompat.getRootWindowInsets(window.decorView)?.let { insets ->
      bottomPx = insets.getInsetsIgnoringVisibility(WindowInsetsCompat.Type.navigationBars()).bottom
    }
    val css = (bottomPx / density).roundToInt()
    return (css + 8).coerceAtLeast(24)
  }

  /** 供连接页 / 远程 Web 调用。 */
  inner class MobileBridge {
    @JavascriptInterface
    fun disconnect() {
      runOnUiThread { loadConnectPage() }
    }

    @JavascriptInterface
    fun isRemoteClient(): Boolean = true

    /** 顶栏安全区（CSS 像素）。 */
    @JavascriptInterface
    fun getStatusBarInsetPx(): Int = statusBarInsetCssPx()

    /** 底栏 / 系统导航安全区（CSS 像素）。 */
    @JavascriptInterface
    fun getNavBarInsetPx(): Int = navBarInsetCssPx()

    /** 连接探测成功后调用，提示系统密码管理器保存 URL+Bearer。 */
    @JavascriptInterface
    fun notifyLoginSuccess() {
      runOnUiThread {
        autofillManager()?.commit()
        // 手动连接成功后即将 navigate 到远程；短延迟清 history，降低 goBack 退回裸连接页的窗口。
        appWebView?.postDelayed({ trimHistoryIfRemote() }, 500)
        appWebView?.postDelayed({ trimHistoryIfRemote() }, 1500)
      }
    }

    /** 连接失败时取消本次 Autofill 会话。 */
    @JavascriptInterface
    fun notifyLoginFailure() {
      runOnUiThread {
        autofillManager()?.cancel()
      }
    }
    /** 在系统浏览器中打开 http(s)/mailto（远程 WebView 内 `window.open` 通常无效）。 */
    @JavascriptInterface
    fun openExternalUrl(url: String) {
      runOnUiThread {
        try {
          val uri = android.net.Uri.parse(url.trim())
          val scheme = uri.scheme?.lowercase()
          if (scheme != "http" && scheme != "https" && scheme != "mailto") {
            return@runOnUiThread
          }
          startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, uri))
        } catch (_: Exception) {
          // 无浏览器或非法 URL：静默忽略
        }
      }
    }
  }

  companion object {
    /**
     * 是否为壳内连接页 Origin。按 scheme + host 解析，禁止 `contains("://tauri.localhost")`
     * 子串误判（恶意查询串可污染 [connectHomeUrl]）。
     */
    fun isAppOrigin(url: String?): Boolean {
      if (url.isNullOrBlank()) {
        return true
      }
      return try {
        val uri = android.net.Uri.parse(url)
        when (uri.scheme?.lowercase()) {
          "tauri", "asset" -> true
          "http", "https" -> {
            val host = uri.host?.lowercase() ?: return false
            host == "tauri.localhost" ||
              (host == "localhost" && (uri.path?.contains("connect") == true))
          }
          else -> false
        }
      } catch (_: Exception) {
        false
      }
    }

    fun stripFragmentAndQuery(url: String): String {
      val noFrag = url.substringBefore('#')
      return noFrag.substringBefore('?')
    }
  }
}
