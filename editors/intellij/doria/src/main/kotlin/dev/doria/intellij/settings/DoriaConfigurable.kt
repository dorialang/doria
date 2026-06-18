package dev.doria.intellij.settings

import com.intellij.openapi.options.SearchableConfigurable
import com.intellij.ui.components.JBLabel
import java.awt.GridBagConstraints
import java.awt.GridBagLayout
import java.awt.Insets
import javax.swing.JComponent
import javax.swing.JPanel
import javax.swing.JTextField

class DoriaConfigurable : SearchableConfigurable {
    private var languageServerPathField: JTextField? = null

    override fun getId(): String = "dev.doria.intellij.settings"

    override fun getDisplayName(): String = "Doria"

    override fun createComponent(): JComponent {
        val settings = DoriaSettings.getInstance().state
        val panel = JPanel(GridBagLayout())
        val constraints = GridBagConstraints().apply {
            gridx = 0
            gridy = 0
            anchor = GridBagConstraints.WEST
            insets = Insets(0, 0, 8, 8)
        }

        panel.add(JBLabel("Language server path:"), constraints)

        languageServerPathField = JTextField(settings.languageServerPath, 40)
        constraints.gridx = 1
        constraints.weightx = 1.0
        constraints.fill = GridBagConstraints.HORIZONTAL
        panel.add(languageServerPathField, constraints)

        constraints.gridx = 1
        constraints.gridy = 1
        constraints.insets = Insets(0, 0, 0, 0)
        constraints.fill = GridBagConstraints.NONE
        constraints.weightx = 0.0
        panel.add(
            JBLabel("Leave empty to use DORIA_LSP_PATH, target/debug/doria-lsp, or doria-lsp on PATH."),
            constraints,
        )

        return panel
    }

    override fun isModified(): Boolean =
        languageServerPathField?.text.orEmpty() != DoriaSettings.getInstance().state.languageServerPath

    override fun apply() {
        DoriaSettings.getInstance().state.languageServerPath = languageServerPathField?.text.orEmpty().trim()
    }

    override fun reset() {
        languageServerPathField?.text = DoriaSettings.getInstance().state.languageServerPath
    }

    override fun disposeUIResources() {
        languageServerPathField = null
    }
}
