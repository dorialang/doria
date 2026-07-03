package dev.doria.intellij.lsp

import com.intellij.openapi.vfs.VirtualFile
import dev.doria.intellij.DoriaFileType

object DoriaLspFiles {
    fun isDoriaSourceFile(file: VirtualFile): Boolean =
        isDoriaFile(file) && !isEditorFixture(file)

    private fun isDoriaFile(file: VirtualFile): Boolean =
        file.fileType == DoriaFileType.INSTANCE || file.extension == "doria"

    private fun isEditorFixture(file: VirtualFile): Boolean =
        file.path.replace('\\', '/').contains("/editors/fixtures/")
}
