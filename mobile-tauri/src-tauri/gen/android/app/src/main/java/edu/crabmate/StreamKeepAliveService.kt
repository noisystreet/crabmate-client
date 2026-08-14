package edu.crabmate

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat

/**
 * 对话进行中的 `dataSync` 前台服务（ADR-0002）。
 * 不消费 `/chat/stream`；通知点按只把 [MainActivity] 拉回前台。
 */
class StreamKeepAliveService : Service() {
  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(
    intent: Intent?,
    flags: Int,
    startId: Int,
  ): Int {
    if (intent == null) {
      stopForegroundCompat()
      stopSelf()
      return START_NOT_STICKY
    }
    when (intent.action) {
      ACTION_STOP -> {
        active = false
        awaitingApproval = false
        stopForegroundCompat()
        stopSelf()
        return START_NOT_STICKY
      }

      ACTION_APPROVAL -> {
        awaitingApproval = true
        commandPreview = intent.getStringExtra(EXTRA_PREVIEW).orEmpty()
        localeSlug = intent.getStringExtra(EXTRA_LOCALE).orEmpty().ifBlank { localeSlug }
      }

      ACTION_CLEAR_APPROVAL -> {
        awaitingApproval = false
        commandPreview = ""
      }

      else -> {
        active = true
        localeSlug = intent.getStringExtra(EXTRA_LOCALE).orEmpty().ifBlank { localeSlug }
      }
    }
    if (!active) {
      return START_NOT_STICKY
    }
    ensureChannels()
    val notification = buildNotification()
    try {
      if (Build.VERSION.SDK_INT >= 34) {
        startForeground(
          NOTIFICATION_ID,
          notification,
          ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
        )
      } else {
        startForeground(NOTIFICATION_ID, notification)
      }
    } catch (_: Exception) {
      active = false
      awaitingApproval = false
      stopSelf()
      return START_NOT_STICKY
    }
    return START_NOT_STICKY
  }

  override fun onDestroy() {
    stopForegroundCompat()
    active = false
    awaitingApproval = false
    super.onDestroy()
  }

  override fun onTaskRemoved(rootIntent: Intent?) {
    // 流未结束时划掉 Recents 仍保持 FGS（Manifest `stopWithTask=false`）。
    if (!active) {
      stopSelf()
    }
  }

  private fun stopForegroundCompat() {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
      stopForeground(STOP_FOREGROUND_REMOVE)
    } else {
      @Suppress("DEPRECATION")
      stopForeground(true)
    }
  }

  private fun ensureChannels() {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
      return
    }
    val mgr = getSystemService(NotificationManager::class.java) ?: return
    mgr.createNotificationChannel(
      NotificationChannel(
        CHANNEL_STREAM,
        getString(R.string.stream_keepalive_channel_stream),
        NotificationManager.IMPORTANCE_DEFAULT,
      ),
    )
    mgr.createNotificationChannel(
      NotificationChannel(
        CHANNEL_APPROVAL,
        getString(R.string.stream_keepalive_channel_approval),
        NotificationManager.IMPORTANCE_HIGH,
      ),
    )
  }

  private fun buildNotification(): Notification {
    val english = StreamKeepAliveText.isEnglish(localeSlug)
    val launch =
      PendingIntent.getActivity(
        this,
        0,
        Intent(this, MainActivity::class.java).apply {
          flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or
            Intent.FLAG_ACTIVITY_CLEAR_TOP or
            Intent.FLAG_ACTIVITY_REORDER_TO_FRONT
        },
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
      )
    val channel = if (awaitingApproval) CHANNEL_APPROVAL else CHANNEL_STREAM
    val title =
      if (awaitingApproval) {
        if (english) {
          getString(R.string.stream_keepalive_approval_title_en)
        } else {
          getString(R.string.stream_keepalive_approval_title)
        }
      } else if (english) {
        getString(R.string.stream_keepalive_stream_title_en)
      } else {
        getString(R.string.stream_keepalive_stream_title)
      }
    val text =
      if (awaitingApproval) {
        commandPreview.ifBlank {
          if (english) {
            getString(R.string.stream_keepalive_approval_body_en)
          } else {
            getString(R.string.stream_keepalive_approval_body)
          }
        }
      } else if (english) {
        getString(R.string.stream_keepalive_stream_body_en)
      } else {
        getString(R.string.stream_keepalive_stream_body)
      }
    val icon =
      if (awaitingApproval) {
        android.R.drawable.stat_sys_warning
      } else {
        android.R.drawable.stat_notify_sync
      }
    return NotificationCompat
      .Builder(this, channel)
      .setSmallIcon(icon)
      .setContentTitle(title)
      .setContentText(text)
      .setStyle(NotificationCompat.BigTextStyle().bigText(text))
      .setContentIntent(launch)
      .setOngoing(true)
      .setOnlyAlertOnce(!awaitingApproval)
      .setPriority(
        if (awaitingApproval) {
          NotificationCompat.PRIORITY_HIGH
        } else {
          NotificationCompat.PRIORITY_DEFAULT
        },
      ).setCategory(NotificationCompat.CATEGORY_STATUS)
      .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
      .build()
  }

  companion object {
    const val CHANNEL_STREAM: String = "crabmate.stream"
    const val CHANNEL_APPROVAL: String = "crabmate.stream.approval"
    const val NOTIFICATION_ID: Int = 42001

    const val ACTION_START: String = "edu.crabmate.action.STREAM_KEEPALIVE_START"
    const val ACTION_STOP: String = "edu.crabmate.action.STREAM_KEEPALIVE_STOP"
    const val ACTION_APPROVAL: String = "edu.crabmate.action.STREAM_KEEPALIVE_APPROVAL"
    const val ACTION_CLEAR_APPROVAL: String = "edu.crabmate.action.STREAM_KEEPALIVE_CLEAR_APPROVAL"

    private const val EXTRA_LOCALE: String = "locale"
    private const val EXTRA_PREVIEW: String = "preview"

    @Volatile
    var active: Boolean = false
      private set

    @Volatile
    private var awaitingApproval: Boolean = false

    @Volatile
    private var localeSlug: String = ""

    @Volatile
    private var commandPreview: String = ""

    fun hasNotifyPermission(context: Context): Boolean {
      if (Build.VERSION.SDK_INT < 33) {
        return true
      }
      return ContextCompat.checkSelfPermission(
        context,
        android.Manifest.permission.POST_NOTIFICATIONS,
      ) == PackageManager.PERMISSION_GRANTED
    }

    fun start(
      context: Context,
      locale: String,
    ) {
      active = true
      val intent =
        Intent(context, StreamKeepAliveService::class.java).apply {
          action = ACTION_START
          putExtra(EXTRA_LOCALE, locale)
        }
      startServiceCompat(context, intent)
    }

    fun stop(context: Context) {
      val app = context.applicationContext
      active = false
      awaitingApproval = false
      commandPreview = ""
      app.stopService(Intent(app, StreamKeepAliveService::class.java))
    }

    fun notifyApproval(
      context: Context,
      command: String,
      args: String,
      locale: String,
    ) {
      if (!active) {
        // 禁止在后台新起 FGS（Android 12+）；attach 时应已 start。
        return
      }
      val intent =
        Intent(context, StreamKeepAliveService::class.java).apply {
          action = ACTION_APPROVAL
          putExtra(EXTRA_LOCALE, locale)
          putExtra(EXTRA_PREVIEW, StreamKeepAliveText.preview(command, args))
        }
      startServiceCompat(context, intent)
    }

    fun clearApproval(context: Context) {
      if (!active) {
        return
      }
      val intent =
        Intent(context, StreamKeepAliveService::class.java).apply {
          action = ACTION_CLEAR_APPROVAL
        }
      startServiceCompat(context, intent)
    }

    private fun startServiceCompat(
      context: Context,
      intent: Intent,
    ) {
      val app = context.applicationContext
      try {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
          app.startForegroundService(intent)
        } else {
          app.startService(intent)
        }
      } catch (_: Exception) {
        active = false
        awaitingApproval = false
        commandPreview = ""
      }
    }
  }
}