package dev.doria.intellij.actions

import com.intellij.icons.AllIcons
import com.intellij.ide.actions.CreateFileFromTemplateAction
import com.intellij.ide.fileTemplates.FileTemplateManager
import com.intellij.openapi.actionSystem.ActionUpdateThread
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.LangDataKeys
import com.intellij.openapi.command.WriteCommandAction
import com.intellij.openapi.project.DumbAwareAction
import com.intellij.openapi.project.Project
import com.intellij.openapi.ui.DialogWrapper
import com.intellij.openapi.ui.Messages
import com.intellij.openapi.ui.ValidationInfo
import com.intellij.psi.PsiDirectory
import com.intellij.psi.PsiFile
import com.intellij.ui.DocumentAdapter
import com.intellij.ui.TitledSeparator
import com.intellij.ui.components.JBLabel
import com.intellij.ui.components.JBList
import com.intellij.ui.components.JBScrollPane
import com.intellij.ui.components.JBTextField
import com.intellij.util.IncorrectOperationException
import com.intellij.util.PathUtilRt
import dev.doria.intellij.DoriaIcons
import java.awt.BorderLayout
import java.awt.Dimension
import java.awt.FlowLayout
import java.awt.GridBagConstraints
import java.awt.GridBagLayout
import java.awt.Insets
import javax.swing.DefaultListModel
import javax.swing.JButton
import javax.swing.JComponent
import javax.swing.JPanel
import javax.swing.ListSelectionModel
import javax.swing.event.DocumentEvent

class DoriaCreateClassAction : DumbAwareAction(
    "Doria Class",
    "Create a Doria class",
    DoriaIcons.FILE,
) {
    override fun getActionUpdateThread(): ActionUpdateThread = ActionUpdateThread.EDT

    override fun update(event: AnActionEvent) {
        event.presentation.isEnabledAndVisible =
            event.project != null && event.getData(LangDataKeys.IDE_VIEW) != null
    }

    override fun actionPerformed(event: AnActionEvent) {
        val project = event.project ?: return
        val view = event.getData(LangDataKeys.IDE_VIEW) ?: return
        val directory = view.orChooseDirectory ?: return
        val dialog = DoriaCreateClassDialog(project, directory)
        if (!dialog.showAndGet()) return

        try {
            val created = WriteCommandAction.writeCommandAction(project)
                .withName("Create Doria class ${dialog.className}")
                .compute<PsiFile?, RuntimeException> {
                    createFromTemplate(project, directory, dialog)
                }
            created?.let(view::selectElement)
        } catch (error: IncorrectOperationException) {
            Messages.showErrorDialog(
                project,
                error.message ?: "The Doria class could not be created.",
                "Cannot Create Doria Class",
            )
        }
    }

    private fun createFromTemplate(
        project: Project,
        directory: PsiDirectory,
        dialog: DoriaCreateClassDialog,
    ): PsiFile? {
        val template = FileTemplateManager.getInstance(project)
            .getInternalTemplate(DORIA_CLASS_TEMPLATE)
        return CreateFileFromTemplateAction.createFileFromTemplate(
            dialog.fileBaseName,
            template,
            directory,
            null,
            true,
            emptyMap(),
            mapOf(
                "NAMESPACE_DECLARATION" to dialog.namespaceDeclaration,
                "CLASS_NAME" to dialog.className,
                "INHERITANCE" to dialog.inheritanceClause,
            ),
        )
    }

    private companion object {
        const val DORIA_CLASS_TEMPLATE = "Doria Class"
    }
}

private class DoriaCreateClassDialog(
    private val project: Project,
    private val directory: PsiDirectory,
) : DialogWrapper(project, true) {
    private val classNameField = JBTextField(42)
    private val namespaceField = JBTextField(42)
    private val fileNameField = JBTextField(42)
    private val directoryField = JBTextField(directory.virtualFile.presentableUrl, 42)
    private val parentField = JBTextField(42)
    private val interfaceModel = DefaultListModel<String>()
    private val interfaceList = JBList(interfaceModel)
    private val addInterfaceButton = JButton(AllIcons.General.Add)
    private val removeInterfaceButton = JButton(AllIcons.General.Remove)
    private var updatingFileName = false
    private var fileNameWasEdited = false

    val className: String
        get() = classNameField.text.trim()

    val fileBaseName: String
        get() = fileNameField.text.trim().removeSuffix(".doria")

    val namespaceDeclaration: String
        get() = namespaceField.text.trim().let { namespace ->
            if (namespace.isEmpty()) "" else "namespace $namespace;\n\n"
        }

    val inheritanceClause: String
        get() = buildString {
            val parent = parentField.text.trim()
            if (parent.isNotEmpty()) append(" extends ").append(parent)

            val interfaces = interfaces()
            if (interfaces.isNotEmpty()) {
                append(" implements ").append(interfaces.joinToString(", "))
            }
        }

    init {
        title = "Create New Doria Class"
        setOKButtonText("OK")
        directoryField.isEditable = false
        classNameField.emptyText.text = "Class name"
        namespaceField.emptyText.text = "Optional namespace"
        fileNameField.emptyText.text = "ClassName.doria"
        parentField.emptyText.text = "Optional parent class"
        interfaceList.emptyText.text = "Choose interfaces to implement"
        interfaceList.selectionMode = ListSelectionModel.SINGLE_SELECTION
        interfaceList.addListSelectionListener {
            removeInterfaceButton.isEnabled = interfaceList.selectedIndex >= 0
        }
        addInterfaceButton.toolTipText = "Add interface"
        addInterfaceButton.accessibleContext.accessibleName = "Add interface"
        addInterfaceButton.addActionListener { addInterface() }
        removeInterfaceButton.toolTipText = "Remove selected interface"
        removeInterfaceButton.accessibleContext.accessibleName = "Remove selected interface"
        removeInterfaceButton.isEnabled = false
        removeInterfaceButton.addActionListener { removeSelectedInterface() }
        installFileNameSynchronization()
        init()
    }

    override fun createCenterPanel(): JComponent = JPanel(GridBagLayout()).apply {
        addSection(this, 0, "Class")
        addRow(this, 1, "Name:", classNameField)
        addRow(this, 2, "Namespace:", namespaceField)
        addRow(this, 3, "File name:", fileNameField)
        addRow(this, 4, "Directory:", directoryField)
        addSection(this, 5, "Parent classes")
        addRow(this, 6, "Extends:", parentField)

        add(
            JBLabel("Implements:"),
            GridBagConstraints().apply {
                gridx = 0
                gridy = 7
                anchor = GridBagConstraints.FIRST_LINE_START
                insets = Insets(8, 20, 4, 12)
            },
        )
        add(
            interfacePanel(),
            GridBagConstraints().apply {
                gridx = 1
                gridy = 7
                weightx = 1.0
                weighty = 1.0
                fill = GridBagConstraints.BOTH
                insets = Insets(4, 0, 4, 0)
            },
        )
    }

    override fun getPreferredFocusedComponent(): JComponent = classNameField

    override fun getInitialSize(): Dimension = Dimension(720, 500)

    override fun doValidate(): ValidationInfo? {
        if (!isDoriaClassName(className)) {
            return ValidationInfo("Enter a valid Doria class name.", classNameField)
        }
        if (fileBaseName.isEmpty() || !PathUtilRt.isValidFileName("$fileBaseName.doria", true)) {
            return ValidationInfo("Enter a valid Doria file name.", fileNameField)
        }
        if (directory.findFile("$fileBaseName.doria") != null) {
            return ValidationInfo("$fileBaseName.doria already exists.", fileNameField)
        }

        val namespace = namespaceField.text.trim()
        if (namespace.isNotEmpty() && !isDoriaQualifiedName(namespace)) {
            return ValidationInfo("Enter a valid Doria namespace.", namespaceField)
        }
        val parent = parentField.text.trim()
        if (parent.isNotEmpty() && !isDoriaQualifiedName(parent)) {
            return ValidationInfo("Enter one valid Doria parent type.", parentField)
        }
        return null
    }

    private fun installFileNameSynchronization() {
        classNameField.document.addDocumentListener(object : DocumentAdapter() {
            override fun textChanged(event: DocumentEvent) {
                if (fileNameWasEdited) return
                updatingFileName = true
                val name = classNameField.text.trim()
                fileNameField.text = if (name.isEmpty()) "" else "$name.doria"
                updatingFileName = false
            }
        })
        fileNameField.document.addDocumentListener(object : DocumentAdapter() {
            override fun textChanged(event: DocumentEvent) {
                if (!updatingFileName) fileNameWasEdited = true
            }
        })
    }

    private fun addInterface() {
        val interfaceName = Messages.showInputDialog(
            project,
            "Interface name:",
            "Add Doria Interface",
            DoriaIcons.FILE,
        )?.trim() ?: return
        if (!isDoriaQualifiedName(interfaceName)) {
            Messages.showErrorDialog(
                project,
                "Enter a valid Doria interface type.",
                "Invalid Interface",
            )
            return
        }
        if (interfaces().contains(interfaceName)) {
            Messages.showErrorDialog(
                project,
                "$interfaceName is already selected.",
                "Duplicate Interface",
            )
            return
        }
        interfaceModel.addElement(interfaceName)
        interfaceList.selectedIndex = interfaceModel.size() - 1
    }

    private fun removeSelectedInterface() {
        val index = interfaceList.selectedIndex
        if (index >= 0) interfaceModel.remove(index)
    }

    private fun interfaces(): List<String> =
        (0 until interfaceModel.size()).map(interfaceModel::getElementAt)

    private fun interfacePanel(): JComponent = JPanel(BorderLayout(0, 4)).apply {
        add(
            JPanel(FlowLayout(FlowLayout.LEFT, 0, 0)).apply {
                add(addInterfaceButton)
                add(removeInterfaceButton)
            },
            BorderLayout.NORTH,
        )
        add(JBScrollPane(interfaceList), BorderLayout.CENTER)
    }

    private fun addSection(panel: JPanel, row: Int, title: String) {
        panel.add(
            TitledSeparator(title),
            GridBagConstraints().apply {
                gridx = 0
                gridy = row
                gridwidth = 2
                weightx = 1.0
                fill = GridBagConstraints.HORIZONTAL
                insets = Insets(if (row == 0) 0 else 12, 0, 4, 0)
            },
        )
    }

    private fun addRow(panel: JPanel, row: Int, label: String, field: JComponent) {
        panel.add(
            JBLabel(label),
            GridBagConstraints().apply {
                gridx = 0
                gridy = row
                anchor = GridBagConstraints.LINE_START
                insets = Insets(4, 20, 4, 12)
            },
        )
        panel.add(
            field,
            GridBagConstraints().apply {
                gridx = 1
                gridy = row
                weightx = 1.0
                fill = GridBagConstraints.HORIZONTAL
                insets = Insets(4, 0, 4, 0)
            },
        )
    }

    private companion object {
        val DORIA_IDENTIFIER = Regex("[A-Za-z_][A-Za-z0-9_]*")
        val DORIA_QUALIFIED_NAME = Regex("""[A-Za-z_][A-Za-z0-9_]*(?:\\[A-Za-z_][A-Za-z0-9_]*)*""")
        val DORIA_RESERVED_NAME_SEGMENTS = setOf(
            "class", "interface", "implements", "namespace", "extends", "function",
            "internal", "static", "let", "take", "writable", "readonly", "return", "echo",
            "new", "foreach", "as", "if", "else", "while", "for", "break", "continue",
            "throw", "throws", "true", "false", "null", "void", "int", "int8", "int16",
            "int32", "int64", "uint8", "uint16", "uint32", "uint64", "float", "float32",
            "float64", "string", "bool", "not", "and", "or", "xor", "async", "await",
            "spawn", "scope", "trait", "enum", "match", "try", "catch", "mixed", "never",
            "resource", "array", "object",
        )
        val DORIA_RESERVED_CLASS_NAMES = setOf(
            "Int", "Int8", "Int16", "Int32", "UInt8", "UInt16", "UInt32", "UInt64",
            "Float", "Float32", "Float64", "Bool", "Displayable",
        )

        fun isDoriaIdentifier(value: String): Boolean =
            DORIA_IDENTIFIER.matches(value) && value !in DORIA_RESERVED_NAME_SEGMENTS

        fun isDoriaClassName(value: String): Boolean =
            isDoriaIdentifier(value) &&
                value !in DORIA_RESERVED_CLASS_NAMES &&
                !value.equals("__DoriaDisplayable", ignoreCase = true)

        fun isDoriaQualifiedName(value: String): Boolean =
            DORIA_QUALIFIED_NAME.matches(value) && value.split('\\').all(::isDoriaIdentifier)
    }
}
