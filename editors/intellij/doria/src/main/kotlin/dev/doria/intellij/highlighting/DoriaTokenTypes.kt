package dev.doria.intellij.highlighting

import com.intellij.psi.TokenType
import com.intellij.psi.tree.IElementType
import dev.doria.intellij.DoriaLanguage

class DoriaTokenType(debugName: String) : IElementType(debugName, DoriaLanguage)

object DoriaTokenTypes {
    val WHITE_SPACE: IElementType = TokenType.WHITE_SPACE
    val BAD_CHARACTER: IElementType = TokenType.BAD_CHARACTER

    val KEYWORD = DoriaTokenType("DORIA_KEYWORD")
    val MODIFIER = DoriaTokenType("DORIA_MODIFIER")
    val PRIMITIVE_TYPE = DoriaTokenType("DORIA_PRIMITIVE_TYPE")
    val COLLECTION_TYPE = DoriaTokenType("DORIA_COLLECTION_TYPE")
    val TYPE_NAME = DoriaTokenType("DORIA_TYPE_NAME")
    val IMPORT_USE_KEYWORD = DoriaTokenType("DORIA_IMPORT_USE_KEYWORD")
    val IMPORT_PATH = DoriaTokenType("DORIA_IMPORT_PATH")
    val IMPORT_ALIAS_KEYWORD = DoriaTokenType("DORIA_IMPORT_ALIAS_KEYWORD")
    val IMPORT_ALIAS = DoriaTokenType("DORIA_IMPORT_ALIAS")
    val TRAIT_USE_KEYWORD = DoriaTokenType("DORIA_TRAIT_USE_KEYWORD")
    val TRAIT_NAME = DoriaTokenType("DORIA_TRAIT_NAME")
    val FUNCTION_DECLARATION = DoriaTokenType("DORIA_FUNCTION_DECLARATION")
    val FUNCTION_CALL = DoriaTokenType("DORIA_FUNCTION_CALL")
    val METHOD_CALL = DoriaTokenType("DORIA_METHOD_CALL")
    val STATIC_METHOD_CALL = DoriaTokenType("DORIA_STATIC_METHOD_CALL")
    val IDENTIFIER = DoriaTokenType("DORIA_IDENTIFIER")
    val VARIABLE = DoriaTokenType("DORIA_VARIABLE")
    val THIS = DoriaTokenType("DORIA_THIS")
    val BOOLEAN_LITERAL = DoriaTokenType("DORIA_BOOLEAN_LITERAL")
    val NULL_LITERAL = DoriaTokenType("DORIA_NULL_LITERAL")
    val NUMBER = DoriaTokenType("DORIA_NUMBER")
    val STRING = DoriaTokenType("DORIA_STRING")
    val ESCAPE_SEQUENCE = DoriaTokenType("DORIA_ESCAPE_SEQUENCE")
    val COMMENT = DoriaTokenType("DORIA_COMMENT")
    val DOC_COMMENT = DoriaTokenType("DORIA_DOC_COMMENT")
    val DOC_COMMENT_TAG = DoriaTokenType("DORIA_DOC_COMMENT_TAG")
    val INVALID = DoriaTokenType("DORIA_INVALID")
    val OPERATOR = DoriaTokenType("DORIA_OPERATOR")
    val BRACE = DoriaTokenType("DORIA_BRACE")
    val BRACKET = DoriaTokenType("DORIA_BRACKET")
    val PAREN = DoriaTokenType("DORIA_PAREN")
    val PUNCTUATION = DoriaTokenType("DORIA_PUNCTUATION")
}
