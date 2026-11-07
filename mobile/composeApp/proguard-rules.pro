# S2: keep the generated UniFFI classes for JNA reflection and for the app.
-keep class uniffi.** { *; }

# JNA is reflection-heavy; these are the rules JNA documents for Android.
-dontwarn java.awt.**
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
