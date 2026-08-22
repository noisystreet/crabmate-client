package edu.crabmate

import android.os.Build
import android.os.Bundle
import android.view.View
import android.view.WindowManager
import android.view.autofill.AutofillManager
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AlertDialog
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import kotlin.math.roundToInt

class MainActivity : TauriActivity() {
  /** 与 Tauri Android 默认资产源一致（`useHttpsScheme=false` → http）。启动页为包内 connect.html。 */
  private var connectHomeUrl: String = DEFAULT_CONNECT_HOME
  private var appWebView: WebView? = null
  private var exitConfirmDialog: AlertDialog? = null

  /** 流式 attach 期间为 true：`onPause` 后仍 `resumeTimers`，避免 SSE 被冻。 */
  private var streamKeepAliveWanted: Boolean = false
  private var lastKeepAliveLocale: String = ""

  private val notifyPermissionLauncher =
    registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
      notifyPermissionDenied = !granted
      if (streamKeepAliveWanted && granted) {
        StreamKeepAliveService.start(this, lastKeepAliveLocale)
      }
      notifyWebKeepAlivePermission(granted)
    }

  /** 用户已明确拒绝通知权限（弹出系统框期间不算）。 */
  @Volatile
  private var notifyPermissionDenied: Boolean = false

  /**
   * 仅在 UI 线程采样的当前 WebView URL。
   *
   * `@JavascriptInterface` 跑在 WebView 后台线程，**禁止**在那里调用 `WebView.getUrl()`
   *（非线程安全，常返回 null/脏值 → 误拒 Keystore 读写，表现为保存失败或重启丢密钥）。
   */
  @Volatile
  private var cachedWebViewUrl: String? = null

  private val chatImageShare = ChatImageShare()
  private val deviceFileShare = ChatImageShare()
  private val pendingDeviceSave by lazy { PendingDeviceSave(cacheDir) }

  private val createDocumentLauncher =
    registerForActivityResult(CreateNamedDocument()) { uri ->
      pendingDeviceSave.complete(uri, applicationContext.contentResolver)
    }

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
    trimHistoryIfNotConnectPage()
    appWebView?.post { refreshCachedWebViewUrl() }
    resumeWebViewTimersIfKeepAlive()
  }

  override fun onPause() {
    super.onPause()
    // Tauri 基类 pause 可能 pauseTimers；保活期间立刻恢复，让 fetch/SSE 回调继续。
    resumeWebViewTimersIfKeepAlive()
  }

  private fun finishDecodedSave(share: ChatImageShare): Boolean {
    val decoded = share.finish(true) ?: return false
    return queueCreateDocument(decoded.first, decoded.second)
  }

  private fun queueCreateDocument(
    filename: String,
    bytes: ByteArray,
  ): Boolean {
    if (!pendingDeviceSave.tryBeginWrite(bytes)) {
      return false
    }
    val req =
      CreateNamedDocument.Request(
        mime = ChatImageShare.mimeForName(filename),
        displayName = filename,
      )
    runOnUiThread {
      try {
        createDocumentLauncher.launch(req)
      } catch (_: Exception) {
        pendingDeviceSave.abort()
      }
    }
    return true
  }

  /** 必须在 UI 线程调用。 */
  private fun refreshCachedWebViewUrl() {
    val url = appWebView?.url
    if (!url.isNullOrBlank()) {
      cachedWebViewUrl = url
      rememberConnectHomeIfConnectPage(url)
    }
  }

  /** 导航/首屏期间多次采样，供 JS 桥用缓存做 Origin 判定。 */
  private fun scheduleUrlCacheSampling(webView: WebView) {
    for (delayMs in longArrayOf(0L, 50L, 150L, 400L, 1000L, 2500L, 5000L, 10000L)) {
      webView.postDelayed({ refreshCachedWebViewUrl() }, delayMs)
    }
  }

  /** 注册（或重新注册）返回键回调，确保优先于 Tauri AppPlugin 的默认 goBack。 */
  private fun installBackPressedHandler() {
    backPressedCallback?.remove()
    val callback =
      object : OnBackPressedCallback(true) {
        override fun handleOnBackPressed() {
          refreshCachedWebViewUrl()
          val url = cachedWebViewUrl
          if (isConnectPageUrl(url)) {
            // 连接页：确认后退出
            showExitConfirmDialog(offerReturnToConnect = false)
          } else {
            // 包内业务 UI 或（过渡期）远程 serve：可退出，或回到连接页换服务器
            showExitConfirmDialog(offerReturnToConnect = true)
          }
        }
      }
    backPressedCallback = callback
    onBackPressedDispatcher.addCallback(this, callback)
  }

  /**
   * 已离开连接页时清掉可 goBack 的历史（包内 index 或远程 serve）。
   * 若 AppPlugin 仍抢到 Back：canGoBack==false 时会转交 activity.onBackPressed，落到我们的确认框。
   */
  private fun trimHistoryIfNotConnectPage() {
    val view = appWebView ?: return
    refreshCachedWebViewUrl()
    if (!isConnectPageUrl(cachedWebViewUrl) && view.canGoBack()) {
      view.clearHistory()
    }
  }

  /**
   * WebView 就绪后短窗口内多次重钉回调，覆盖 Rust 侧晚到的 AppPlugin 构造。
   * 同时在已离开连接页时 trim history。
   */
  private fun scheduleBackHandlerDominance(webView: WebView) {
    installBackPressedHandler()
    // 含较晚一档：覆盖慢速自动登录 navigate，以及 Rust 侧晚到的 AppPlugin.load。
    for (delayMs in longArrayOf(0L, 100L, 500L, 2000L, 5000L)) {
      webView.postDelayed(
        {
          refreshCachedWebViewUrl()
          installBackPressedHandler()
          trimHistoryIfNotConnectPage()
        },
        delayMs,
      )
    }
  }

  /**
   * 系统返回键确认框。
   * @param offerReturnToConnect 业务 UI 时额外提供「返回连接页」。
   */
  private fun showExitConfirmDialog(offerReturnToConnect: Boolean) {
    if (exitConfirmDialog?.isShowing == true) {
      return
    }
    val builder =
      AlertDialog
        .Builder(this)
        .setTitle(R.string.exit_confirm_title)
        .setMessage(
          if (offerReturnToConnect) {
            R.string.exit_confirm_message_remote
          } else {
            R.string.exit_confirm_message
          },
        ).setNegativeButton(R.string.exit_confirm_cancel, null)
        .setPositiveButton(R.string.exit_confirm_ok) { _, _ ->
          streamKeepAliveWanted = false
          StreamKeepAliveService.stop(this)
          finishAffinity()
        }.setOnDismissListener { exitConfirmDialog = null }
    if (offerReturnToConnect) {
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
    // 与前端 `--bg` 对齐：HTML/WASM 未就绪前避免系统默认纯黑空页。
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
    webView.post { refreshCachedWebViewUrl() }
    scheduleUrlCacheSampling(webView)
    scheduleBackHandlerDominance(webView)
  }

  /**
   * 写入 Web 的安全区与软键盘 inset。
   * `--cm-ime-inset`：IME 相对导航栏多出的高度（targetSdk 35+ 上 adjustResize 常不缩小窗口时的兜底）。
   */
  private fun injectSafeInsetsCss(
    webView: WebView,
    insets: WindowInsetsCompat?,
  ) {
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

  /** 仅当当前确为连接页时更新 home；勿用包内 index.html 覆盖。 */
  private fun rememberConnectHomeIfConnectPage(url: String?) {
    if (url.isNullOrBlank() || !isConnectPageUrl(url)) {
      return
    }
    connectHomeUrl = stripFragmentAndQuery(url).ifBlank { DEFAULT_CONNECT_HOME }
  }

  private fun loadConnectPage() {
    val view = appWebView ?: return
    // 与工具栏断开一致：回连接页时清除 Keystore 中的连接 Bearer。
    try {
      SecureBearerStore.write(applicationContext, "")
    } catch (_: Exception) {
      // ignore
    }
    streamKeepAliveWanted = false
    StreamKeepAliveService.stop(this)
    // 勿用当前业务 UI URL 覆盖 connectHome（Phase 2 同为 tauri.localhost）。
    val base =
      when {
        isConnectPageUrl(connectHomeUrl) -> stripFragmentAndQuery(connectHomeUrl)
        else -> DEFAULT_CONNECT_HOME
      }
    // ?manual=1：跳过连接页冷启动自动登录，便于更换服务器
    val sep = if (base.contains('?')) '&' else '?'
    view.loadUrl("$base${sep}manual=1")
    view.post { refreshCachedWebViewUrl() }
    // 页面提交后再清后退栈（慢网多试一次）；避免 goBack 退回业务 UI 或误走无 manual 的历史项。
    for (delayMs in longArrayOf(400L, 1200L)) {
      view.postDelayed(
        {
          refreshCachedWebViewUrl()
          if (isConnectPageUrl(cachedWebViewUrl)) {
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

  private fun resumeWebViewTimersIfKeepAlive() {
    if (!streamKeepAliveWanted && !StreamKeepAliveService.active) {
      return
    }
    streamKeepAliveWanted = streamKeepAliveWanted || StreamKeepAliveService.active
    appWebView?.resumeTimers()
  }

  private fun notifyWebKeepAlivePermission(granted: Boolean) {
    val js =
      "try{var f=globalThis.__cmKeepAlivePermission;if(typeof f==='function')f($granted);}catch(e){}"
    appWebView?.evaluateJavascript(js, null)
  }

  private fun maybeRequestNotificationPermission() {
    if (Build.VERSION.SDK_INT < 33) {
      return
    }
    if (StreamKeepAliveService.hasNotifyPermission(this)) {
      return
    }
    notifyPermissionLauncher.launch(android.Manifest.permission.POST_NOTIFICATIONS)
  }

  private fun allowKeepAliveBridge(): Boolean = allowSecureBearerBridge()

  /**
   * 包内 App Origin（连接页或业务 UI）可读写加密连接 Bearer；远程 serve 页拒绝。
   * 缓存未就绪时拒绝（由前端短延迟重试）；业务 UI 需水合/清除，不得仅限连接页。
   */
  private fun allowSecureBearerBridge(): Boolean {
    val url = cachedWebViewUrl
    if (url.isNullOrBlank()) {
      return false
    }
    return isAppOrigin(url)
  }

  /**
   * 包内 App Origin（连接页或业务 UI）可读写模型密钥；远程 serve 页拒绝。
   * 缓存未就绪时拒绝（由前端短延迟重试）；切勿在 JS 桥线程读 `WebView.getUrl()`。
   */
  private fun allowSecureLlmSecretBridge(): Boolean {
    val url = cachedWebViewUrl
    if (url.isNullOrBlank()) {
      return false
    }
    return isAppOrigin(url)
  }

  /** 供连接页 / 包内业务 UI 调用。 */
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

    /**
     * 读取 Keystore AES-GCM 加密的连接 Bearer。包内 App Origin（连接页 / 业务 UI）。
     */
    @JavascriptInterface
    fun getSecureBearer(): String {
      if (!allowSecureBearerBridge()) {
        return ""
      }
      return try {
        SecureBearerStore.read(applicationContext)
      } catch (_: Exception) {
        ""
      }
    }

    /**
     * 写入（或清空）加密 Bearer。包内 App Origin；空串删除条目。
     * @return true 表示已写入/清除；false 表示拒绝或存储不可用
     */
    @JavascriptInterface
    fun setSecureBearer(bearer: String): Boolean {
      if (!allowSecureBearerBridge()) {
        return false
      }
      return try {
        SecureBearerStore.write(applicationContext, bearer)
      } catch (_: Exception) {
        false
      }
    }

    /**
     * 读取 Keystore 加密的模型 API 密钥槽（`client_llm` / `executor_llm` / `saved_models`）。
     * 仅包内 App Origin；远程页返回空串。
     */
    @JavascriptInterface
    fun getSecureLlmSecret(slot: String): String {
      if (!allowSecureLlmSecretBridge()) {
        return ""
      }
      return try {
        SecureLlmSecretStore.read(applicationContext, slot)
      } catch (_: Exception) {
        ""
      }
    }

    /**
     * 写入或清除模型 API 密钥槽。空串删除；仅包内 App Origin。
     */
    @JavascriptInterface
    fun setSecureLlmSecret(
      slot: String,
      value: String,
    ): Boolean {
      if (!allowSecureLlmSecretBridge()) {
        return false
      }
      return try {
        SecureLlmSecretStore.write(applicationContext, slot, value)
      } catch (_: Exception) {
        false
      }
    }

    /** 连接探测成功后调用，提示系统密码管理器保存 URL+Bearer。 */
    @JavascriptInterface
    fun notifyLoginSuccess() {
      runOnUiThread {
        autofillManager()?.commit()
        refreshCachedWebViewUrl()
        // 手动连接成功后即将 navigate 到包内 UI；短延迟清 history。
        appWebView?.postDelayed({
          refreshCachedWebViewUrl()
          trimHistoryIfNotConnectPage()
        }, 500)
        appWebView?.postDelayed({
          refreshCachedWebViewUrl()
          trimHistoryIfNotConnectPage()
        }, 1500)
      }
    }

    /** 连接失败时取消本次 Autofill 会话。 */
    @JavascriptInterface
    fun notifyLoginFailure() {
      runOnUiThread {
        autofillManager()?.cancel()
      }
    }

    /**
     * 流式 attach 开始：拉起 dataSync FGS。
     * 返回 `ok` / `prompting`（系统权限框弹出中）/ `need_permission`（已拒绝）/ 空串（Origin 拒绝）。
     * 须在 Activity 仍前台时调用（ADR-0002）。
     */
    @JavascriptInterface
    fun startStreamKeepAlive(locale: String): String {
      if (!allowKeepAliveBridge()) {
        return ""
      }
      val loc = locale.trim()
      runOnUiThread {
        streamKeepAliveWanted = true
        lastKeepAliveLocale = loc
        maybeRequestNotificationPermission()
        StreamKeepAliveService.start(this@MainActivity, loc)
        resumeWebViewTimersIfKeepAlive()
      }
      return if (StreamKeepAliveService.hasNotifyPermission(applicationContext)) {
        "ok"
      } else if (notifyPermissionDenied) {
        "need_permission"
      } else {
        "prompting"
      }
    }

    @JavascriptInterface
    fun stopStreamKeepAlive() {
      if (!allowKeepAliveBridge()) {
        return
      }
      runOnUiThread {
        streamKeepAliveWanted = false
        StreamKeepAliveService.stop(this@MainActivity)
      }
    }

    /**
     * 升级为审批 heads-up。不把 session id 写入 Intent；点按只打开 Activity。
     */
    @JavascriptInterface
    fun notifyApproval(
      command: String,
      args: String,
      locale: String,
    ) {
      if (!allowKeepAliveBridge()) {
        return
      }
      val cmd = command.take(256)
      val a = args.take(256)
      val loc = locale.trim()
      runOnUiThread {
        streamKeepAliveWanted = true
        lastKeepAliveLocale = loc.ifBlank { lastKeepAliveLocale }
        resumeWebViewTimersIfKeepAlive()
        StreamKeepAliveService.notifyApproval(this@MainActivity, cmd, a, lastKeepAliveLocale)
      }
    }

    @JavascriptInterface
    fun clearApprovalNotification() {
      if (!allowKeepAliveBridge()) {
        return
      }
      runOnUiThread {
        StreamKeepAliveService.clearApproval(this@MainActivity)
      }
    }

    /** 在系统浏览器中打开 http(s)/mailto（WebView 内 `window.open` 通常无效）。 */
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

    /** 聊天灯箱：开始接收分块 base64（仅包内 App Origin）。 */
    @JavascriptInterface
    fun beginChatImageSave(filename: String): Boolean {
      if (!allowSecureBearerBridge()) {
        return false
      }
      return chatImageShare.begin(true, filename, asImage = true)
    }

    /** 工作区「保存到本机」等：系统另存为（WebView 无可靠 `<a download>`）。 */
    @JavascriptInterface
    fun beginDeviceFileSave(filename: String): Boolean {
      if (!allowSecureBearerBridge()) {
        return false
      }
      return deviceFileShare.begin(true, filename, asImage = false)
    }

    @JavascriptInterface
    fun appendDeviceFileSave(chunk: String): Boolean {
      if (!allowSecureBearerBridge()) {
        return false
      }
      return deviceFileShare.append(true, chunk)
    }

    @JavascriptInterface
    fun finishDeviceFileSave(): Boolean {
      if (!allowSecureBearerBridge()) {
        deviceFileShare.cancel()
        return false
      }
      return this@MainActivity.finishDecodedSave(deviceFileShare)
    }

    @JavascriptInterface
    fun cancelDeviceFileSave() {
      if (!allowSecureBearerBridge()) {
        return
      }
      deviceFileShare.cancel()
    }

    @JavascriptInterface
    fun appendChatImageSave(chunk: String): Boolean {
      if (!allowSecureBearerBridge()) {
        return false
      }
      return chatImageShare.append(true, chunk)
    }

    @JavascriptInterface
    fun finishChatImageSave(): Boolean {
      if (!allowSecureBearerBridge()) {
        chatImageShare.cancel()
        return false
      }
      return this@MainActivity.finishDecodedSave(chatImageShare)
    }
  }

  companion object {
    const val DEFAULT_CONNECT_HOME: String = "http://tauri.localhost/connect.html"

    /**
     * 是否为壳内 App 资产 Origin（连接页或包内业务 UI）。
     * 按 scheme + host 解析，禁止 `contains("://tauri.localhost")` 子串误判。
     */
    fun isAppOrigin(url: String?): Boolean {
      if (url.isNullOrBlank()) {
        return true
      }
      return try {
        val uri = android.net.Uri.parse(url)
        when (uri.scheme?.lowercase()) {
          "tauri", "asset" -> {
            true
          }

          "http", "https" -> {
            val host = uri.host?.lowercase() ?: return false
            host == "tauri.localhost" ||
              (host == "localhost" && (uri.path?.contains("connect") == true))
          }

          else -> {
            false
          }
        }
      } catch (_: Exception) {
        false
      }
    }

    /** 是否为连接页（`…/connect.html`）；包内 `index.html` 不算。 */
    fun isConnectPageUrl(url: String?): Boolean {
      if (url.isNullOrBlank() || !isAppOrigin(url)) {
        return false
      }
      return try {
        val path =
          android.net.Uri
            .parse(url)
            .path ?: return false
        path.endsWith("connect.html")
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