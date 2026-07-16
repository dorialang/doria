package dev.doria.intellij.codestyle

import com.intellij.application.options.CodeStyleAbstractConfigurable
import com.intellij.application.options.CodeStyleAbstractPanel
import com.intellij.application.options.IndentOptionsEditor
import com.intellij.application.options.SmartIndentOptionsEditor
import com.intellij.application.options.TabbedLanguageCodeStylePanel
import com.intellij.lang.Language
import com.intellij.psi.codeStyle.CodeStyleConfigurable
import com.intellij.psi.codeStyle.CodeStyleSettings
import com.intellij.psi.codeStyle.CodeStyleSettingsCustomizable
import com.intellij.psi.codeStyle.CommonCodeStyleSettings
import com.intellij.psi.codeStyle.LanguageCodeStyleSettingsProvider
import dev.doria.intellij.DoriaLanguage

class DoriaLanguageCodeStyleSettingsProvider : LanguageCodeStyleSettingsProvider() {
    override fun getLanguage(): Language = DoriaLanguage

    override fun getLanguageName(): String = "Doria"

    override fun getCodeSample(settingsType: SettingsType): String = CODE_SAMPLE

    override fun getIndentOptionsEditor(): IndentOptionsEditor = SmartIndentOptionsEditor()

    override fun createConfigurable(
        settings: CodeStyleSettings,
        modelSettings: CodeStyleSettings,
    ): CodeStyleConfigurable = object : CodeStyleAbstractConfigurable(
        settings,
        modelSettings,
        languageName,
    ) {
        override fun createPanel(settings: CodeStyleSettings): CodeStyleAbstractPanel =
            object : TabbedLanguageCodeStylePanel(
                DoriaLanguage,
                currentSettings,
                settings,
            ) {}
    }

    override fun customizeDefaults(
        commonSettings: CommonCodeStyleSettings,
        indentOptions: CommonCodeStyleSettings.IndentOptions,
    ) {
        indentOptions.INDENT_SIZE = 4
        indentOptions.CONTINUATION_INDENT_SIZE = 4
        indentOptions.TAB_SIZE = 4
        indentOptions.USE_TAB_CHARACTER = false

        commonSettings.RIGHT_MARGIN = 120
        commonSettings.BRACE_STYLE = CommonCodeStyleSettings.END_OF_LINE
        commonSettings.CLASS_BRACE_STYLE = CommonCodeStyleSettings.NEXT_LINE
        commonSettings.METHOD_BRACE_STYLE = CommonCodeStyleSettings.NEXT_LINE
        commonSettings.ELSE_ON_NEW_LINE = false
        commonSettings.CATCH_ON_NEW_LINE = false
        commonSettings.FINALLY_ON_NEW_LINE = false
        commonSettings.KEEP_LINE_BREAKS = true
        commonSettings.LINE_COMMENT_ADD_SPACE = true
    }

    override fun customizeSettings(
        consumer: CodeStyleSettingsCustomizable,
        settingsType: SettingsType,
    ) {
        when (settingsType) {
            SettingsType.SPACING_SETTINGS -> consumer.showStandardOptions(
                "SPACE_AROUND_ASSIGNMENT_OPERATORS",
                "SPACE_AROUND_LOGICAL_OPERATORS",
                "SPACE_AROUND_EQUALITY_OPERATORS",
                "SPACE_AROUND_RELATIONAL_OPERATORS",
                "SPACE_AROUND_BITWISE_OPERATORS",
                "SPACE_AROUND_ADDITIVE_OPERATORS",
                "SPACE_AROUND_MULTIPLICATIVE_OPERATORS",
                "SPACE_AROUND_SHIFT_OPERATORS",
                "SPACE_AROUND_UNARY_OPERATOR",
                "SPACE_AFTER_COMMA",
                "SPACE_BEFORE_COMMA",
                "SPACE_AFTER_SEMICOLON",
                "SPACE_BEFORE_SEMICOLON",
                "SPACE_BEFORE_METHOD_CALL_PARENTHESES",
                "SPACE_BEFORE_METHOD_PARENTHESES",
                "SPACE_BEFORE_IF_PARENTHESES",
                "SPACE_BEFORE_WHILE_PARENTHESES",
                "SPACE_BEFORE_FOR_PARENTHESES",
                "SPACE_BEFORE_CATCH_PARENTHESES",
                "SPACE_WITHIN_PARENTHESES",
                "SPACE_WITHIN_METHOD_CALL_PARENTHESES",
                "SPACE_WITHIN_METHOD_PARENTHESES",
                "SPACE_WITHIN_IF_PARENTHESES",
                "SPACE_WITHIN_WHILE_PARENTHESES",
                "SPACE_WITHIN_FOR_PARENTHESES",
                "SPACE_WITHIN_CATCH_PARENTHESES",
                "SPACE_WITHIN_BRACKETS",
                "SPACE_BEFORE_CLASS_LBRACE",
                "SPACE_BEFORE_METHOD_LBRACE",
                "SPACE_BEFORE_IF_LBRACE",
                "SPACE_BEFORE_ELSE_LBRACE",
                "SPACE_BEFORE_WHILE_LBRACE",
                "SPACE_BEFORE_FOR_LBRACE",
                "SPACE_BEFORE_TRY_LBRACE",
                "SPACE_BEFORE_CATCH_LBRACE",
                "SPACE_BEFORE_FINALLY_LBRACE",
            )

            SettingsType.WRAPPING_AND_BRACES_SETTINGS -> consumer.showStandardOptions(
                "RIGHT_MARGIN",
                "WRAP_ON_TYPING",
                "KEEP_LINE_BREAKS",
                "BRACE_STYLE",
                "CLASS_BRACE_STYLE",
                "METHOD_BRACE_STYLE",
                "ELSE_ON_NEW_LINE",
                "CATCH_ON_NEW_LINE",
                "FINALLY_ON_NEW_LINE",
            )

            SettingsType.BLANK_LINES_SETTINGS -> consumer.showStandardOptions(
                "KEEP_BLANK_LINES_IN_DECLARATIONS",
                "KEEP_BLANK_LINES_IN_CODE",
                "KEEP_BLANK_LINES_BEFORE_RBRACE",
                "BLANK_LINES_AROUND_CLASS",
                "BLANK_LINES_AROUND_FIELD",
                "BLANK_LINES_AROUND_METHOD",
            )

            else -> Unit
        }
    }

    companion object {
        private val CODE_SAMPLE = """
            namespace App\Services;

            class Greeter
            {
                function greet(string ${'$'}name): string
                {
                    if (${'$'}name == "Doria") {
                        return "Hello, {${'$'}name}!";
                    }

                    return "Hello!";
                }
            }
        """.trimIndent()
    }
}
