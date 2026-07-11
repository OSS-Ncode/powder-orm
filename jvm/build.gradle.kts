// Declared once at the root (not activated here) so both sibling subprojects
// share one plugin classloader/build-service instance. Applying the same
// version independently in each subproject's own `plugins {}` block causes
// Gradle to load two separate MavenCentralBuildService instances and fail
// with "Cannot set the value of task ... using a provider ... loaded with"
// once a cross-project task graph (like `publishAndReleaseToMavenCentral`)
// touches both.
plugins {
    // Also declared here (apply false), not just in powder-kotlin: vanniktech's
    // Kotlin-version check does a classloader lookup for KotlinBasePlugin, and
    // that only succeeds if both plugins share the same root-scoped classloader.
    // Declaring Kotlin only in the subproject caused NoClassDefFoundError.
    kotlin("jvm") version "2.2.0" apply false
    id("com.vanniktech.maven.publish") version "0.37.0" apply false
}
