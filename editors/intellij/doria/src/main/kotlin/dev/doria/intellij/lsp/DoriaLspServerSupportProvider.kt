package dev.doria.intellij.lsp

import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.LspServerSupportProvider
import dev.doria.intellij.DoriaFileType

class DoriaLspServerSupportProvider : LspServerSupportProvider {
    override fun fileOpened(project: Project, file: VirtualFile, serverStarter: LspServerSupportProvider.LspServerStarter) {
        if (file.fileType == DoriaFileType.INSTANCE || file.extension == "doria") {
            serverStarter.ensureServerStarted(DoriaLspServerDescriptor(project))
        }
    }
}
