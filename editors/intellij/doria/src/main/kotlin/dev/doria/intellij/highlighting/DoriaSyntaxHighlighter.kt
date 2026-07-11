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
        DoriaTokenTypes.RESERVED_TYPE -> RESERVED_TYPE_KEYS
        DoriaTokenTypes.COLLECTION_TYPE -> COLLECTION_TYPE_KEYS
        DoriaTokenTypes.TYPE_NAME -> TYPE_NAME_KEYS
        DoriaTokenTypes.IMPORT_USE_KEYWORD -> IMPORT_USE_KEYWORD_KEYS
        DoriaTokenTypes.IMPORT_PATH -> IMPORT_PATH_KEYS
        DoriaTokenTypes.IMPORT_ALIAS_KEYWORD -> IMPORT_ALIAS_KEYWORD_KEYS
        DoriaTokenTypes.IMPORT_ALIAS -> IMPORT_ALIAS_KEYS
        DoriaTokenTypes.TRAIT_USES_KEYWORD -> TRAIT_USES_KEYWORD_KEYS
        DoriaTokenTypes.TRAIT_NAME -> TRAIT_NAME_KEYS
        DoriaTokenTypes.ATTRIBUTE_DELIMITER -> ATTRIBUTE_DELIMITER_KEYS
        DoriaTokenTypes.ATTRIBUTE_NAME -> ATTRIBUTE_NAME_KEYS
        DoriaTokenTypes.ATTRIBUTE_ARGUMENT -> ATTRIBUTE_ARGUMENT_KEYS
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
        DoriaTokenTypes.ESCAPE_SEQUENCE -> ESCAPE_SEQUENCE_KEYS
        DoriaTokenTypes.COMMENT -> COMMENT_KEYS
        DoriaTokenTypes.DOC_COMMENT -> DOC_COMMENT_KEYS
        DoriaTokenTypes.DOC_COMMENT_TAG -> DOC_COMMENT_TAG_KEYS
        DoriaTokenTypes.INVALID -> INVALID_KEYS
        DoriaTokenTypes.OPERATOR -> OPERATOR_KEYS
        DoriaTokenTypes.LOGICAL_OPERATOR -> LOGICAL_OPERATOR_KEYS
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
        val RESERVED_TYPE: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_RESERVED_TYPE",
            DefaultLanguageHighlighterColors.METADATA,
        )
        val COLLECTION_TYPE: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_COLLECTION_TYPE",
            DefaultLanguageHighlighterColors.CLASS_NAME,
        )
        val TYPE_NAME: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_TYPE_NAME",
            DefaultLanguageHighlighterColors.CLASS_NAME,
        )
        val IMPORT_USE_KEYWORD: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_IMPORT_USE_KEYWORD",
            DefaultLanguageHighlighterColors.KEYWORD,
        )
        val IMPORT_PATH: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_IMPORT_PATH",
            DefaultLanguageHighlighterColors.CLASS_NAME,
        )
        val IMPORT_ALIAS_KEYWORD: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_IMPORT_ALIAS_KEYWORD",
            DefaultLanguageHighlighterColors.KEYWORD,
        )
        val IMPORT_ALIAS: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_IMPORT_ALIAS",
            DefaultLanguageHighlighterColors.CLASS_NAME,
        )
        val TRAIT_USES_KEYWORD: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_TRAIT_USES_KEYWORD",
            DefaultLanguageHighlighterColors.KEYWORD,
        )
        val TRAIT_NAME: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_TRAIT_NAME",
            DefaultLanguageHighlighterColors.CLASS_NAME,
        )
        val ATTRIBUTE_DELIMITER: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_ATTRIBUTE_DELIMITER",
            DefaultLanguageHighlighterColors.METADATA,
        )
        val ATTRIBUTE_NAME: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_ATTRIBUTE_NAME",
            DefaultLanguageHighlighterColors.METADATA,
        )
        val ATTRIBUTE_ARGUMENT: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_ATTRIBUTE_ARGUMENT",
            DefaultLanguageHighlighterColors.PARAMETER,
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
            DefaultLanguageHighlighterColors.INSTANCE_FIELD,
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
        val ESCAPE_SEQUENCE: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_ESCAPE_SEQUENCE",
            DefaultLanguageHighlighterColors.VALID_STRING_ESCAPE,
        )
        val COMMENT: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_COMMENT",
            DefaultLanguageHighlighterColors.LINE_COMMENT,
        )
        val DOC_COMMENT: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_DOC_COMMENT",
            DefaultLanguageHighlighterColors.DOC_COMMENT,
        )
        val DOC_COMMENT_TAG: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_DOC_COMMENT_TAG",
            DefaultLanguageHighlighterColors.DOC_COMMENT_TAG,
        )
        val INVALID: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_INVALID",
            HighlighterColors.BAD_CHARACTER,
        )
        val OPERATOR: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_OPERATOR",
            DefaultLanguageHighlighterColors.OPERATION_SIGN,
        )
        val LOGICAL_OPERATOR: TextAttributesKey = TextAttributesKey.createTextAttributesKey(
            "DORIA_LOGICAL_OPERATOR",
            DefaultLanguageHighlighterColors.KEYWORD,
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
        private val RESERVED_TYPE_KEYS = arrayOf(RESERVED_TYPE)
        private val COLLECTION_TYPE_KEYS = arrayOf(COLLECTION_TYPE)
        private val TYPE_NAME_KEYS = arrayOf(TYPE_NAME)
        private val IMPORT_USE_KEYWORD_KEYS = arrayOf(IMPORT_USE_KEYWORD)
        private val IMPORT_PATH_KEYS = arrayOf(IMPORT_PATH)
        private val IMPORT_ALIAS_KEYWORD_KEYS = arrayOf(IMPORT_ALIAS_KEYWORD)
        private val IMPORT_ALIAS_KEYS = arrayOf(IMPORT_ALIAS)
        private val TRAIT_USES_KEYWORD_KEYS = arrayOf(TRAIT_USES_KEYWORD)
        private val TRAIT_NAME_KEYS = arrayOf(TRAIT_NAME)
        private val ATTRIBUTE_DELIMITER_KEYS = arrayOf(ATTRIBUTE_DELIMITER)
        private val ATTRIBUTE_NAME_KEYS = arrayOf(ATTRIBUTE_NAME)
        private val ATTRIBUTE_ARGUMENT_KEYS = arrayOf(ATTRIBUTE_ARGUMENT)
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
        private val ESCAPE_SEQUENCE_KEYS = arrayOf(ESCAPE_SEQUENCE)
        private val COMMENT_KEYS = arrayOf(COMMENT)
        private val DOC_COMMENT_KEYS = arrayOf(DOC_COMMENT)
        private val DOC_COMMENT_TAG_KEYS = arrayOf(DOC_COMMENT_TAG)
        private val INVALID_KEYS = arrayOf(INVALID)
        private val OPERATOR_KEYS = arrayOf(OPERATOR)
        private val LOGICAL_OPERATOR_KEYS = arrayOf(LOGICAL_OPERATOR)
        private val BRACE_KEYS = arrayOf(BRACE)
        private val BRACKET_KEYS = arrayOf(BRACKET)
        private val PAREN_KEYS = arrayOf(PAREN)
        private val PUNCTUATION_KEYS = arrayOf(PUNCTUATION)
        private val BAD_CHARACTER_KEYS = arrayOf(BAD_CHARACTER)
    }
}
