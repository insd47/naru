# JNA finds the core's functions, structure fields, and callbacks by name through reflection, so the generated bindings
# and JNA itself keep their names.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class dev.insd.naru.core.** { *; }

# JNA refers to desktop Java classes Android does not have.
-dontwarn java.awt.**
