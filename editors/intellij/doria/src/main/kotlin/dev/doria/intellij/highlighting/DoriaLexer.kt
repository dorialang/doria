package dev.doria.intellij.highlighting

import com.intellij.lexer.LexerBase
import com.intellij.psi.tree.IElementType

class DoriaLexer : LexerBase() {
    private var buffer: CharSequence = ""
    private var startOffset: Int = 0
    private var endOffset: Int = 0
    private var tokenStart: Int = 0
    private var tokenEnd: Int = 0
    private var tokenType: IElementType? = null

    override fun start(buffer: CharSequence, startOffset: Int, endOffset: Int, initialState: Int) {
        this.buffer = buffer
        this.startOffset = startOffset
        this.endOffset = endOffset
        this.tokenStart = startOffset
        this.tokenEnd = startOffset
        advance()
    }

    override fun getState(): Int = 0

    override fun getTokenType(): IElementType? = tokenType

    override fun getTokenStart(): Int = tokenStart

    override fun getTokenEnd(): Int = tokenEnd

    override fun getBufferSequence(): CharSequence = buffer

    override fun getBufferEnd(): Int = endOffset

    override fun advance() {
        tokenStart = tokenEnd

        if (tokenStart >= endOffset) {
            tokenType = null
            return
        }

        val current = buffer[tokenStart]
        when {
            current.isWhitespace() -> scanWhitespace()
            current == '/' && peek(1) == '/' -> scanLineComment()
            current == '#' && peek(1) != '[' -> scanLineComment()
            current == '/' && peek(1) == '*' -> scanBlockComment()
            current == '"' || current == '\'' -> scanString(current)
            current == '$' -> scanVariable()
            current.isDigit() -> scanNumber()
            isIdentifierStart(current) -> scanIdentifierLike()
            else -> scanSymbol()
        }
    }

    private fun scanWhitespace() {
        tokenEnd = tokenStart + 1
        while (tokenEnd < endOffset && buffer[tokenEnd].isWhitespace()) {
            tokenEnd++
        }
        tokenType = DoriaTokenTypes.WHITE_SPACE
    }

    private fun scanLineComment() {
        tokenEnd = tokenStart + 1
        while (tokenEnd < endOffset && buffer[tokenEnd] != '\n' && buffer[tokenEnd] != '\r') {
            tokenEnd++
        }
        tokenType = DoriaTokenTypes.COMMENT
    }

    private fun scanBlockComment() {
        tokenEnd = tokenStart + 2
        while (tokenEnd < endOffset) {
            if (buffer[tokenEnd - 1] == '*' && buffer[tokenEnd] == '/') {
                tokenEnd++
                break
            }
            tokenEnd++
        }
        tokenType = DoriaTokenTypes.COMMENT
    }

    private fun scanString(quote: Char) {
        tokenEnd = tokenStart + 1
        var escaped = false
        while (tokenEnd < endOffset) {
            val char = buffer[tokenEnd]
            tokenEnd++
            if (escaped) {
                escaped = false
                continue
            }
            if (char == '\\') {
                escaped = true
                continue
            }
            if (char == quote) {
                break
            }
        }
        tokenType = DoriaTokenTypes.STRING
    }

    private fun scanVariable() {
        tokenEnd = tokenStart + 1
        if (tokenEnd < endOffset && isIdentifierStart(buffer[tokenEnd])) {
            tokenEnd++
            while (tokenEnd < endOffset && isIdentifierPart(buffer[tokenEnd])) {
                tokenEnd++
            }
            tokenType = if (sliceEquals("$this")) DoriaTokenTypes.THIS else DoriaTokenTypes.VARIABLE
        } else {
            tokenType = DoriaTokenTypes.OPERATOR
        }
    }

    private fun scanNumber() {
        tokenEnd = tokenStart + 1
        while (tokenEnd < endOffset && buffer[tokenEnd].isDigit()) {
            tokenEnd++
        }
        if (tokenEnd + 1 < endOffset && buffer[tokenEnd] == '.' && buffer[tokenEnd + 1].isDigit()) {
            tokenEnd++
            while (tokenEnd < endOffset && buffer[tokenEnd].isDigit()) {
                tokenEnd++
            }
        }
        tokenType = DoriaTokenTypes.NUMBER
    }

    private fun scanIdentifierLike() {
        tokenEnd = tokenStart + 1
        while (tokenEnd < endOffset && isIdentifierPart(buffer[tokenEnd])) {
            tokenEnd++
        }

        val text = buffer.subSequence(tokenStart, tokenEnd).toString()
        tokenType = when (text) {
            "class", "function", "let", "return", "echo", "new", "foreach", "as",
            "if", "else", "while", "for", "static", "async", "await", "spawn", "scope",
            "interface", "trait", "enum", "match", "try", "catch", "throw" -> DoriaTokenTypes.KEYWORD

            "writable", "readonly", "internal", "public", "protected", "private" -> DoriaTokenTypes.MODIFIER

            "void", "int", "float", "string", "bool", "mixed", "object", "resource", "array" -> DoriaTokenTypes.PRIMITIVE_TYPE

            "List", "Dictionary", "Set", "Result", "Option" -> DoriaTokenTypes.COLLECTION_TYPE

            "true", "false" -> DoriaTokenTypes.BOOLEAN_LITERAL

            "null" -> DoriaTokenTypes.NULL_LITERAL

            else -> if (text.first().isUpperCase()) DoriaTokenTypes.TYPE_NAME else DoriaTokenTypes.IDENTIFIER
        }
    }

    private fun scanSymbol() {
        val two = take(2)
        val three = take(3)

        tokenEnd = when {
            three in THREE_CHAR_OPERATORS -> tokenStart + 3
            two in TWO_CHAR_OPERATORS -> tokenStart + 2
            else -> tokenStart + 1
        }

        tokenType = when (buffer[tokenStart]) {
            '{', '}' -> DoriaTokenTypes.BRACE
            '[', ']' -> DoriaTokenTypes.BRACKET
            '(', ')' -> DoriaTokenTypes.PAREN
            ';', ',', ':' -> DoriaTokenTypes.PUNCTUATION
            else -> DoriaTokenTypes.OPERATOR
        }
    }

    private fun peek(delta: Int): Char? {
        val index = tokenStart + delta
        return if (index < endOffset) buffer[index] else null
    }

    private fun take(length: Int): String {
        val end = (tokenStart + length).coerceAtMost(endOffset)
        return buffer.subSequence(tokenStart, end).toString()
    }

    private fun sliceEquals(value: String): Boolean {
        val length = tokenEnd - tokenStart
        return length == value.length && buffer.subSequence(tokenStart, tokenEnd).toString() == value
    }

    private fun isIdentifierStart(char: Char): Boolean = char == '_' || char.isLetter()

    private fun isIdentifierPart(char: Char): Boolean = char == '_' || char.isLetterOrDigit()

    companion object {
        private val THREE_CHAR_OPERATORS = setOf("===", "!==")
        private val TWO_CHAR_OPERATORS = setOf("==", "!=", "<=", ">=", "&&", "||", "??", "=>", "+=", "-=", "->", "::")
    }
}
