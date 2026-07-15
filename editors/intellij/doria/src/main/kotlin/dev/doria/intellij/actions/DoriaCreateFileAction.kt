package dev.doria.intellij.actions

import com.intellij.ide.actions.CreateFileFromTemplateAction
import com.intellij.ide.actions.CreateFileFromTemplateDialog
import com.intellij.openapi.project.Project
import com.intellij.psi.PsiDirectory
import dev.doria.intellij.DoriaIcons

class DoriaCreateFileAction : CreateFileFromTemplateAction(
    "Doria File",
    "Create a Doria file with a main function",
    DoriaIcons.FILE,
) {
    override fun buildDialog(
        project: Project,
        directory: PsiDirectory,
        builder: CreateFileFromTemplateDialog.Builder,
    ) {
        builder
            .setTitle("New Doria File")
            .addKind("Doria File", DoriaIcons.FILE, DORIA_FILE_TEMPLATE)
    }

    override fun getActionName(
        directory: PsiDirectory,
        newName: String,
        templateName: String,
    ): String = "Create Doria file $newName"

    private companion object {
        const val DORIA_FILE_TEMPLATE = "Doria File"
    }
}
