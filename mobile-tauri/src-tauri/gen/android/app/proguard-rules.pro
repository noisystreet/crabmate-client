# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# CrabMateMobile JavascriptInterface（release minify 必须保留）
-keepclassmembers class edu.crabmate.MainActivity$MobileBridge {
   @android.webkit.JavascriptInterface <methods>;
}
-keep class edu.crabmate.MainActivity$MobileBridge { *; }

# Wry 通过虚方法回调壳 Activity；勿混淆 override
-keep class edu.crabmate.MainActivity { *; }
-keep class edu.crabmate.SecureBearerStore { *; }
-keepclassmembers class edu.crabmate.WryActivity {
   public <init>(...);
   void setWebView(...);
   void onWebViewCreate(...);
   int getId();
   java.lang.String getVersion();
}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile
