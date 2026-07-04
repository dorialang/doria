package dev.doria.intellij.lsp

import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.LspServerSupportProvider

class DoriaLspServerSupportProvider : LspServerSupportProvider {
    override fun fileOpened(project: Project, file: VirtualFile, serverStarter: LspServerSupportProvider.LspServerStarter) {
        if (DoriaLspFiles.isDoriaSourceFile(file)) {
            serverStarter.ensureServerStarted(DoriaLspServerDescriptor(project))
        }
    }
}
