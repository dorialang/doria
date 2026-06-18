package dev.doria.intellij.highlighting

import com.intellij.lexer.Lexer
import com.intellij.openapi.editor.DefaultLanguageHighlighterColors
import com.intellij.openapi.editor.HighlighterColors
import com.intellij.openapi.editor.colors.CodeInsightColors
import com.intellij.openapi.editor.colors.TextAttributesKey
import com.intellij.openapi.fileTypes.SyntaxHighlighter
import com.intellij.psi.tree.IElementType

class DoriaSyntaxHighlighter : SyntaxHighlighter {
    override fun getHighlightingLexer(): Lexer = DoriaLexer()

    override fun getTokenHighlights(tokenType: IElementType): Array<TextAttributesKey> = when (tokenType) {
        DoriaTokenTypes.KEYWORD -> KEYWORD_KEYS
        DoriaTokenTypes.MODIFIER -> MODIFIER_KEYS
        DoriaTokenTypes.PRIMITIVE_TYPE -> PRIMITIVE_TYPE_KEYS
        DoriaTokenTypes.COLLECTION_TYPE -> COLLECTION_TYPE_KEYS
        DoriaTokenTypes.TYPE_NAME -> TYPE_NAME_KEYS
        DoriaTokenTypes.FUNCTION_DECLARATION -> FUNCTION_DECLARATION_KEYS
        DoriaTokenTypes.FUNCTION_CALL -> FUNCTION_CALL_KEYS
        DoriaTokenTypes.METHOD_CALL -> METHOD_CALL_KEYS
        DoriaTokenTypes.STATIC_METHOD_CALL -> STATIC_METHOD_CALL_KEYS
        DoriaTokenTypes.IDENTIFIER -> IDENTIFIER_KEYS
        DoriaTokenTypes.VARIABLE -> VARIABLE_KEYS
        DoriaTokenTypes.THIS -> THIS_KEYS
        DoriaTokenTypes.BOOLEAN_LITERAL,
        DoriaTokenTypes.NULL_LITERAL -> CONSTANT_KEYS
        DoriaTokenTypes.NUMBER -> NUMBER_KEYS
        DoriaTokenTypes.STRING -> STRING_KEYS
        DoriaTokenTypes.COMMENT -> COMMENT_KEYS
        DoriaTokenTypes.OPERATOR -> OPERATOR_KEYS
        DoriaTokenTypes.BRACE -> BRACE_KEYS
        DoriaTokenTypes.BRACKET -> BRACKET_KEYS
        DoriaTokenTypes.PAREN -> PAREN_KEYS
        DoriaTokenTypes.PUNCTUATION -> PUNCTUATION_KEYS
        DoriaTokenTypes.BAD_CHARACTER -> BAD_CHARACTER_KEYS
        else -> EMPTY_KEYS
    }

    companion object {
        val KEYWORD: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_KEYWORD",
            DefaultLanguageHighlighterColors.KEYWORD,
        )
        val MODIFIER: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_MODIFIER",
            DefaultLanguageHighlighterColors.KEYWORD,
        )
        val PRIMITIVE_TYPE: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_PRIMITIVE_TYPE",
            DefaultLanguageHighlighterColors.KEYWORD,
        )
        val COLLECTION_TYPE: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_COLLECTION_TYPE",
            DefaultLanguageHighlighterColors.CLASS_NAME,
        )
        val TYPE_NAME: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_TYPE_NAME",
            DefaultLanguageHighlighterColors.CLASS_NAME,
        )
        val FUNCTION_DECLARATION: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_FUNCTION_DECLARATION",
            DefaultLanguageHighlighterColors.FUNCTION_DECLARATION,
        )
        val FUNCTION_CALL: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_FUNCTION_CALL",
            DefaultLanguageHighlighterColors.FUNCTION_CALL,
        )
        val METHOD_CALL: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_METHOD_CALL",
            DefaultLanguageHighlighterColors.INSTANCE_METHOD,
        )
        val STATIC_METHOD_CALL: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_STATIC_METHOD_CALL",
            DefaultLanguageHighlighterColors.STATIC_METHOD,
        )
        val IDENTIFIER: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_IDENTIFIER",
            DefaultLanguageHighlighterColors.IDENTIFIER,
        )
        val VARIABLE: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_VARIABLE",
            DefaultLanguageHighlighterColors.LOCAL_VARIABLE,
        )
        val THIS: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_THIS",
            DefaultLanguageHighlighterColors.INSTANCE_FIELD,
        )
        val UNUSED_VARIABLE: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_UNUSED_VARIABLE",
            CodeInsightColors.NOT_USED_ELEMENT_ATTRIBUTES,
        )
        val CONSTANT: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_CONSTANT",
            DefaultLanguageHighlighterColors.CONSTANT,
        )
        val NUMBER: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_NUMBER",
            DefaultLanguageHighlighterColors.NUMBER,
        )
        val STRING: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_STRING",
            DefaultLanguageHighlighterColors.STRING,
        )
        val COMMENT: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_COMMENT",
            DefaultLanguageHighlighterColors.LINE_COMMENT,
        )
        val OPERATOR: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_OPERATOR",
            DefaultLanguageHighlighterColors.OPERATION_SIGN,
        )
        val BRACE: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_BRACE",
            DefaultLanguageHighlighterColors.BRACES,
        )
        val BRACKET: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_BRACKET",
            DefaultLanguageHighlighterColors.BRACKETS,
        )
        val PAREN: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_PAREN",
            DefaultLanguageHighlighterColors.PARENTHESES,
        )
        val PUNCTUATION: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_PUNCTUATION",
            DefaultLanguageHighlighterColors.COMMA,
        )
        val BAD_CHARACTER: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_BAD_CHARACTER",
            HighlighterColors.BAD_CHARACTER,
        )

        private val EMPTY_KEYS = emptyArray<TextAttributesKey>()
        private val KEYWORD_KEYS = arrayOf(KEYWORD)
        private val MODIFIER_KEYS = arrayOf(MODIFIER)
        private val PRIMITIVE_TYPE_KEYS = arrayOf(PRIMITIVE_TYPE)
        private val COLLECTION_TYPE_KEYS = arrayOf(COLLECTION_TYPE)
        private val TYPE_NAME_KEYS = arrayOf(TYPE_NAME)
        private val FUNCTION_DECLARATION_KEYS = arrayOf(FUNCTION_DECLARATION)
        private val FUNCTION_CALL_KEYS = arrayOf(FUNCTION_CALL)
        private val METHOD_CALL_KEYS = arrayOf(METHOD_CALL)
        private val STATIC_METHOD_CALL_KEYS = arrayOf(STATIC_METHOD_CALL)
        private val IDENTIFIER_KEYS = arrayOf(IDENTIFIER)
        private val VARIABLE_KEYS = arrayOf(VARIABLE)
        private val THIS_KEYS = arrayOf(THIS)
        private val CONSTANT_KEYS = arrayOf(CONSTANT)
        private val NUMBER_KEYS = arrayOf(NUMBER)
        private val STRING_KEYS = arrayOf(STRING)
        private val COMMENT_KEYS = arrayOf(COMMENT)
        private val OPERATOR_KEYS = arrayOf(OPERATOR)
        private val BRACE_KEYS = arrayOf(BRACE)
        private val BRACKET_KEYS = arrayOf(BRACKET)
        private val PAREN_KEYS = arrayOf(PAREN)
        private val PUNCTUATION_KEYS = arrayOf(PUNCTUATION)
        private val BAD_CHARACTER_KEYS = arrayOf(BAD_CHARACTER)
    }
}
