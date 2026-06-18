package dev.doria.intellij.lsp

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.ProjectWideLspServerDescriptor
import dev.doria.intellij.DoriaFileType

class DoriaLspServerDescriptor(project: Project) : ProjectWideLspServerDescriptor(project, "Doria") {
    override fun isSupportedFile(file: VirtualFile): Boolean =
        file.fileType == DoriaFileType.INSTANCE || file.extension == "doria"

    override fun createCommandLine(): GeneralCommandLine {
        val commandLine = GeneralCommandLine(DoriaLspServerPathResolver.resolve(project))
        project.basePath?.let { commandLine.withWorkDirectory(it) }
        return commandLine
    }
}
