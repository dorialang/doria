package dev.doria.intellij

import com.intellij.openapi.fileTypes.LanguageFileType
import javax.swing.Icon

class DoriaFileType private constructor() : LanguageFileType(DoriaLanguage) {
    override fun getName(): String = "Doria"

    override fun getDescription(): String = "Doria source file"

    override fun getDefaultExtension(): String = "doria"

    override fun getIcon(): Icon = DoriaIcons.FILE

    companion object {
        @JvmField
        val INSTANCE = DoriaFileType()
    }
}
