package dev.doria.intellij

import com.intellij.openapi.util.IconLoader
import javax.swing.Icon

object DoriaIcons {
    @JvmField
    val FILE: Icon = IconLoader.getIcon("/icons/doria.svg", DoriaIcons::class.java)
}
