plugins {
    id("java")
    id("org.jetbrains.kotlin.jvm") version "1.9.25"
    id("org.jetbrains.intellij") version "1.17.4"
}

group = "dev.doria"
version = "0.1.0"

repositories {
    mavenCentral()
}

intellij {
    version.set("2025.2.1")
    type.set("IU")
}

tasks {
    patchPluginXml {
        sinceBuild.set("252")
    }

    buildSearchableOptions {
        enabled = false
    }

    withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile> {
        kotlinOptions.jvmTarget = "21"
    }
}
