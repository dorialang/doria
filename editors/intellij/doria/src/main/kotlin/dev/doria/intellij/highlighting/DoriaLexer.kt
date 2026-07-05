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
    private var mode: Int = MODE_NORMAL

    override fun start(buffer: CharSequence, startOffset: Int, endOffset: Int, initialState: Int) {
        this.buffer = buffer
        this.startOffset = startOffset
        this.endOffset = endOffset
        this.tokenStart = startOffset
        this.tokenEnd = startOffset
        this.mode = initialState
        advance()
    }

    override fun getState(): Int = mode

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

        when (mode) {
            MODE_DOUBLE_STRING -> {
                scanDoubleStringToken()
                return
            }
            MODE_SINGLE_STRING -> {
                scanSingleStringToken()
                return
            }
            MODE_INTERPOLATION -> {
                scanInterpolationToken()
                return
            }
            MODE_DOC_COMMENT -> {
                scanDocCommentToken()
                return
            }
        }

        scanCodeToken(doubleQuoteStartsInterpolatedString = true)
    }

    private fun scanCodeToken(doubleQuoteStartsInterpolatedString: Boolean) {
        val current = buffer[tokenStart]
        when {
            current.isWhitespace() -> scanWhitespace()
            current == '/' && peek(1) == '/' -> scanLineComment()
            current == '#' && isPreprocessorDirective() -> scanPreprocessorDirective()
            current == '#' && peek(1) != '[' -> scanLineComment()
            current == '/' && peek(1) == '*' && peek(2) == '*' && peek(3) != '/' -> scanDocCommentStart()
            current == '/' && peek(1) == '*' -> scanBlockComment()
            current == '"' && doubleQuoteStartsInterpolatedString -> {
                mode = MODE_DOUBLE_STRING
                scanDoubleStringToken(contentStart = tokenStart + 1)
            }
            current == '"' -> {
                mode = MODE_DOUBLE_STRING
                scanDoubleStringToken(contentStart = tokenStart + 1)
            }
            current == '\'' -> {
                mode = MODE_SINGLE_STRING
                scanSingleStringToken(contentStart = tokenStart + 1)
            }
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

    private fun scanPreprocessorDirective() {
        tokenEnd = tokenStart + 1
        while (tokenEnd < endOffset && buffer[tokenEnd] != '\n' && buffer[tokenEnd] != '\r') {
            tokenEnd++
        }
        tokenType = DoriaTokenTypes.INVALID
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

    private fun scanDocCommentStart() {
        tokenEnd = tokenStart + 3
        tokenType = DoriaTokenTypes.DOC_COMMENT
        mode = MODE_DOC_COMMENT
    }

    private fun scanDocCommentToken() {
        if (buffer[tokenStart] == '*' && peek(1) == '/') {
            tokenEnd = tokenStart + 2
            tokenType = DoriaTokenTypes.DOC_COMMENT
            mode = MODE_NORMAL
            return
        }

        if (buffer[tokenStart] == '@' && peek(1)?.let(::isIdentifierStart) == true) {
            scanDocCommentTag()
            return
        }

        if (buffer[tokenStart] == '$') {
            scanVariable()
            return
        }

        if (isIdentifierStart(buffer[tokenStart])) {
            scanDocCommentIdentifier()
            return
        }

        tokenEnd = tokenStart + 1
        while (tokenEnd < endOffset) {
            if (buffer[tokenEnd] == '*' && tokenEnd + 1 < endOffset && buffer[tokenEnd + 1] == '/') {
                break
            }
            if (buffer[tokenEnd] == '@' && tokenEnd + 1 < endOffset && isIdentifierStart(buffer[tokenEnd + 1])) {
                break
            }
            if (buffer[tokenEnd] == '$' || isIdentifierStart(buffer[tokenEnd])) {
                break
            }
            tokenEnd++
        }
        tokenType = DoriaTokenTypes.DOC_COMMENT
    }

    private fun scanDocCommentTag() {
        tokenEnd = tokenStart + 1
        while (tokenEnd < endOffset && (isIdentifierPart(buffer[tokenEnd]) || buffer[tokenEnd] == '-')) {
            tokenEnd++
        }
        tokenType = DoriaTokenTypes.DOC_COMMENT_TAG
    }

    private fun scanDocCommentIdentifier() {
        tokenEnd = tokenStart + 1
        while (tokenEnd < endOffset && isIdentifierPart(buffer[tokenEnd])) {
            tokenEnd++
        }

        val text = buffer.subSequence(tokenStart, tokenEnd).toString()
        tokenType = if (isDocTypePosition()) {
            when (text) {
                in PRIMITIVE_TYPES -> DoriaTokenTypes.PRIMITIVE_TYPE
                in COLLECTION_TYPES -> DoriaTokenTypes.COLLECTION_TYPE
                else -> if (text.first().isUpperCase()) DoriaTokenTypes.TYPE_NAME else DoriaTokenTypes.DOC_COMMENT
            }
        } else if (text == "static" && isDocMethodStaticModifierPosition()) {
            DoriaTokenTypes.MODIFIER
        } else if (isDocMethodNamePosition()) {
            DoriaTokenTypes.FUNCTION_DECLARATION
        } else {
            DoriaTokenTypes.DOC_COMMENT
        }
    }

    private fun scanSingleStringToken(contentStart: Int = tokenStart) {
        tokenEnd = contentStart
        while (tokenEnd < endOffset) {
            when (buffer[tokenEnd]) {
                '\\' -> {
                    if (tokenEnd == tokenStart) {
                        scanEscapeSequence()
                    } else {
                        tokenType = DoriaTokenTypes.STRING
                    }
                    return
                }
                '\'' -> {
                    tokenEnd++
                    mode = MODE_NORMAL
                    tokenType = DoriaTokenTypes.STRING
                    return
                }
                else -> tokenEnd++
            }
        }

        tokenType = DoriaTokenTypes.STRING
    }

    private fun scanDoubleStringToken(contentStart: Int = tokenStart) {
        tokenEnd = contentStart
        while (tokenEnd < endOffset) {
            val char = buffer[tokenEnd]
            if (char == '\\') {
                if (tokenEnd == tokenStart) {
                    scanEscapeSequence()
                } else {
                    tokenType = DoriaTokenTypes.STRING
                }
                return
            }
            if (char == '{' && tokenEnd + 1 < endOffset && buffer[tokenEnd + 1] == '$') {
                if (tokenEnd == contentStart) {
                    tokenEnd++
                    tokenType = DoriaTokenTypes.STRING
                    mode = MODE_INTERPOLATION
                } else {
                    tokenType = DoriaTokenTypes.STRING
                }
                return
            }
            tokenEnd++
            if (char == '"') {
                mode = MODE_NORMAL
                tokenType = DoriaTokenTypes.STRING
                return
            }
        }

        tokenType = DoriaTokenTypes.STRING
    }

    private fun scanEscapeSequence() {
        tokenEnd = (tokenStart + 2).coerceAtMost(endOffset)
        tokenType = DoriaTokenTypes.ESCAPE_SEQUENCE
    }

    private fun scanInterpolationToken() {
        when {
            buffer[tokenStart] == '}' -> {
                tokenEnd = tokenStart + 1
                tokenType = DoriaTokenTypes.STRING
                mode = MODE_DOUBLE_STRING
            }
            buffer[tokenStart] == '$' -> scanVariable()
            buffer[tokenStart] == '-' && peek(1) == '>' -> {
                tokenEnd = tokenStart + 2
                tokenType = DoriaTokenTypes.OPERATOR
            }
            isIdentifierStart(buffer[tokenStart]) -> scanIdentifierLike()
            buffer[tokenStart].isWhitespace() -> scanWhitespace()
            else -> {
                tokenEnd = tokenStart + 1
                tokenType = DoriaTokenTypes.OPERATOR
            }
        }
    }

    private fun scanVariable() {
        tokenEnd = tokenStart + 1
        if (tokenEnd < endOffset && isIdentifierStart(buffer[tokenEnd])) {
            tokenEnd++
            while (tokenEnd < endOffset && isIdentifierPart(buffer[tokenEnd])) {
                tokenEnd++
            }
            tokenType = if (sliceEquals("\$this")) DoriaTokenTypes.THIS else DoriaTokenTypes.VARIABLE
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
            in INVALID_KEYWORDS -> DoriaTokenTypes.INVALID

            "use" -> useTokenType()

            "uses" -> usesTokenType()

            "as" -> if (isImportUseLine()) DoriaTokenTypes.IMPORT_ALIAS_KEYWORD else DoriaTokenTypes.KEYWORD

            else -> contextualIdentifierTokenType(text)
        }
    }

    private fun contextualIdentifierTokenType(text: String): IElementType = when {
        isTraitUsesLine() && text.first().isUpperCase() -> DoriaTokenTypes.TRAIT_NAME

        isImportAliasName() -> DoriaTokenTypes.IMPORT_ALIAS

        isImportUseLine() && text !in KEYWORDS -> DoriaTokenTypes.IMPORT_PATH

        else -> baseIdentifierTokenType(text)
    }

    private fun baseIdentifierTokenType(text: String): IElementType =
        when (text) {
            in KEYWORDS -> DoriaTokenTypes.KEYWORD

            in WORD_OPERATORS -> DoriaTokenTypes.OPERATOR

            in MODIFIERS -> DoriaTokenTypes.MODIFIER

            in PRIMITIVE_TYPES -> DoriaTokenTypes.PRIMITIVE_TYPE

            in COLLECTION_TYPES -> DoriaTokenTypes.COLLECTION_TYPE

            "true", "false" -> DoriaTokenTypes.BOOLEAN_LITERAL

            "null" -> DoriaTokenTypes.NULL_LITERAL

            else -> when {
                isFunctionDeclarationName() -> DoriaTokenTypes.FUNCTION_DECLARATION
                text.first().isUpperCase() -> DoriaTokenTypes.TYPE_NAME
                nextNonWhitespace(tokenEnd) == '(' -> callableTokenType()
                else -> DoriaTokenTypes.IDENTIFIER
            }
        }

    private fun useTokenType(): IElementType = when {
        isLegacyTraitUseLine() -> DoriaTokenTypes.INVALID
        isImportUseLine() -> DoriaTokenTypes.IMPORT_USE_KEYWORD
        else -> DoriaTokenTypes.KEYWORD
    }

    private fun usesTokenType(): IElementType = when {
        isTraitUsesLine() -> DoriaTokenTypes.TRAIT_USES_KEYWORD
        else -> DoriaTokenTypes.KEYWORD
    }

    private fun scanSymbol() {
        val two = take(2)
        val three = take(3)
        val symbolText = when {
            three in THREE_CHAR_OPERATORS -> three
            two in TWO_CHAR_OPERATORS -> two
            else -> take(1)
        }

        tokenEnd = when {
            three in THREE_CHAR_OPERATORS -> tokenStart + 3
            two in TWO_CHAR_OPERATORS -> tokenStart + 2
            else -> tokenStart + 1
        }

        tokenType = when {
            symbolText in STRICT_COMPARISON_OPERATORS -> DoriaTokenTypes.INVALID
            buffer[tokenStart] == '{' || buffer[tokenStart] == '}' -> DoriaTokenTypes.BRACE
            buffer[tokenStart] == '[' || buffer[tokenStart] == ']' -> DoriaTokenTypes.BRACKET
            buffer[tokenStart] == '(' || buffer[tokenStart] == ')' -> DoriaTokenTypes.PAREN
            buffer[tokenStart] == ';' || buffer[tokenStart] == ',' || buffer[tokenStart] == ':' -> DoriaTokenTypes.PUNCTUATION
            else -> DoriaTokenTypes.OPERATOR
        }
    }

    private fun isPreprocessorDirective(): Boolean {
        if (peek(1) == '[') {
            return false
        }

        var firstNonWhitespace = lineStart(tokenStart)
        while (firstNonWhitespace < tokenStart && buffer[firstNonWhitespace].isWhitespace()) {
            firstNonWhitespace++
        }
        if (firstNonWhitespace != tokenStart) {
            return false
        }

        var cursor = tokenStart + 1
        while (cursor < endOffset && buffer[cursor].isLetter()) {
            cursor++
        }

        if (cursor == tokenStart + 1) {
            return false
        }

        val name = buffer.subSequence(tokenStart + 1, cursor).toString()
        if (name !in PREPROCESSOR_DIRECTIVES) {
            return false
        }

        return cursor >= endOffset || !isIdentifierPart(buffer[cursor])
    }

    private fun isTraitUsesLine(): Boolean =
        TRAIT_USES_LINE.matches(currentLine())

    private fun isLegacyTraitUseLine(): Boolean =
        LEGACY_TRAIT_USE_LINE.matches(currentLine())

    private fun isImportUseLine(): Boolean =
        IMPORT_USE_LINE.matches(currentLine())

    private fun isImportAliasName(): Boolean =
        isImportUseLine() && previousIdentifier() == "as"

    private fun currentLine(): String =
        buffer.subSequence(lineStart(tokenStart), lineEnd(tokenStart)).toString()

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

    private fun isFunctionDeclarationName(): Boolean =
        nextNonWhitespace(tokenEnd) == '(' && previousIdentifier() == "function"

    private fun callableTokenType(): IElementType = when (previousAccessor()) {
        "->" -> DoriaTokenTypes.METHOD_CALL
        "::" -> DoriaTokenTypes.STATIC_METHOD_CALL
        else -> DoriaTokenTypes.FUNCTION_CALL
    }

    private fun previousAccessor(): String? {
        var cursor = tokenStart - 1
        while (cursor >= startOffset && buffer[cursor].isWhitespace()) {
            cursor--
        }
        if (cursor <= startOffset) {
            return null
        }

        val twoChars = buffer.subSequence(cursor - 1, cursor + 1).toString()
        return twoChars.takeIf { it == "->" || it == "::" }
    }

    private fun isDocTypePosition(): Boolean {
        val tag = docTagBefore(tokenStart) ?: return false
        if (tag.name !in DOC_TYPE_TAGS) {
            return false
        }

        val typeRange = docTypeRange(tag.endOffset, lineEnd(tokenStart), tag.name) ?: return false
        return tokenStart in typeRange
    }

    private fun isDocMethodStaticModifierPosition(): Boolean {
        val tag = docTagBefore(tokenStart) ?: return false
        if (tag.name != "method") {
            return false
        }

        var cursor = tag.endOffset
        val lineEnd = lineEnd(tokenStart)
        while (cursor < lineEnd && buffer[cursor].isWhitespace()) {
            cursor++
        }
        return cursor == tokenStart
    }

    private fun isDocMethodNamePosition(): Boolean {
        val tag = docTagBefore(tokenStart) ?: return false
        if (tag.name != "method") {
            return false
        }

        val typeRange = docTypeRange(tag.endOffset, lineEnd(tokenStart), tag.name) ?: return false
        var cursor = typeRange.last + 1
        while (cursor < endOffset && buffer[cursor].isWhitespace()) {
            cursor++
        }
        return cursor == tokenStart && nextNonWhitespace(tokenEnd) == '('
    }

    private fun docTagBefore(index: Int): DocTag? {
        var cursor = lineStart(index)
        var tag: DocTag? = null
        while (cursor < index) {
            if (buffer[cursor] == '@' && cursor + 1 < index && isIdentifierStart(buffer[cursor + 1])) {
                var tagEnd = cursor + 2
                while (tagEnd < index && (isIdentifierPart(buffer[tagEnd]) || buffer[tagEnd] == '-')) {
                    tagEnd++
                }
                tag = DocTag(buffer.subSequence(cursor + 1, tagEnd).toString(), tagEnd)
                cursor = tagEnd
            } else {
                cursor++
            }
        }
        return tag
    }

    private fun docTypeRange(startIndex: Int, lineEnd: Int, tagName: String): IntRange? {
        var cursor = startIndex
        while (cursor < lineEnd && buffer[cursor].isWhitespace()) {
            cursor++
        }
        if (tagName == "method" && hasWordAt(cursor, "static")) {
            cursor += "static".length
            while (cursor < lineEnd && buffer[cursor].isWhitespace()) {
                cursor++
            }
        }
        if (cursor >= lineEnd) {
            return null
        }

        val typeStart = cursor
        var depth = 0
        var sawTypeCharacter = false
        while (cursor < lineEnd) {
            val char = buffer[cursor]
            when {
                char == '$' && depth == 0 -> break
                char == '<' -> {
                    depth++
                    sawTypeCharacter = true
                    cursor++
                }
                char == '>' -> {
                    if (depth > 0) {
                        depth--
                    }
                    sawTypeCharacter = true
                    cursor++
                }
                char.isWhitespace() -> {
                    if (sawTypeCharacter && depth == 0) {
                        break
                    }
                    cursor++
                }
                isDocTypeCharacter(char) -> {
                    sawTypeCharacter = true
                    cursor++
                }
                else -> break
            }
        }

        return if (sawTypeCharacter) typeStart until cursor else null
    }

    private fun lineStart(index: Int): Int {
        var cursor = index - 1
        while (cursor >= startOffset && buffer[cursor] != '\n' && buffer[cursor] != '\r') {
            cursor--
        }
        return cursor + 1
    }

    private fun lineEnd(index: Int): Int {
        var cursor = index
        while (cursor < endOffset && buffer[cursor] != '\n' && buffer[cursor] != '\r') {
            cursor++
        }
        return cursor
    }

    private fun isDocTypeCharacter(char: Char): Boolean =
        isIdentifierPart(char) || char == '\\' || char == '?' || char == '|' || char == '&' ||
            char == '[' || char == ']' || char == ',' || char == '.'

    private fun hasWordAt(index: Int, word: String): Boolean {
        if (index + word.length > endOffset) {
            return false
        }
        if (buffer.subSequence(index, index + word.length).toString() != word) {
            return false
        }

        val before = index - 1
        val after = index + word.length
        val validBefore = before < startOffset || !isIdentifierPart(buffer[before])
        val validAfter = after >= endOffset || !isIdentifierPart(buffer[after])
        return validBefore && validAfter
    }

    private fun nextNonWhitespace(index: Int): Char? {
        var cursor = index
        while (cursor < endOffset && buffer[cursor].isWhitespace()) {
            cursor++
        }
        return if (cursor < endOffset) buffer[cursor] else null
    }

    private fun previousIdentifier(): String? {
        var cursor = tokenStart - 1
        while (cursor >= startOffset && buffer[cursor].isWhitespace()) {
            cursor--
        }
        if (cursor < startOffset || !isIdentifierPart(buffer[cursor])) {
            return null
        }

        val end = cursor + 1
        while (cursor >= startOffset && isIdentifierPart(buffer[cursor])) {
            cursor--
        }

        val start = cursor + 1
        if (start >= end || !isIdentifierStart(buffer[start])) {
            return null
        }
        return buffer.subSequence(start, end).toString()
    }

    private fun isIdentifierStart(char: Char): Boolean = char == '_' || char.isLetter()

    private fun isIdentifierPart(char: Char): Boolean = char == '_' || char.isLetterOrDigit()

    private data class DocTag(val name: String, val endOffset: Int)

    companion object {
        private const val MODE_NORMAL = 0
        private const val MODE_DOUBLE_STRING = 1
        private const val MODE_INTERPOLATION = 2
        const val MODE_DOC_COMMENT = 3
        private const val MODE_SINGLE_STRING = 4

        private val KEYWORDS = setOf(
            "class",
            "interface",
            "trait",
            "extends",
            "implements",
            "function",
            "let",
            "return",
            "echo",
            "new",
            "foreach",
            "as",
            "if",
            "else",
            "while",
            "for",
            "break",
            "continue",
            "when",
            "given",
            "finally",
            "namespace",
            "use",
            "uses",
            "include",
            "declare",
            "static",
            "async",
            "await",
            "spawn",
            "scope",
            "enum",
            "match",
            "try",
            "catch",
            "throw",
            "throws",
        )

        private val WORD_OPERATORS = setOf("not", "and", "or", "xor")

        private val INVALID_KEYWORDS = setOf("goto")

        private val MODIFIERS = setOf("writable", "readonly", "internal")

        private val PRIMITIVE_TYPES = setOf(
            "void",
            "int",
            "int8",
            "int16",
            "int32",
            "int64",
            "uint8",
            "uint16",
            "uint32",
            "uint64",
            "float",
            "float32",
            "float64",
            "string",
            "bool",
            "mixed",
            "object",
            "resource",
            "array",
        )

        private val COLLECTION_TYPES = setOf("List", "Dictionary", "Set")

        private val TRAIT_USES_LINE =
            Regex("^\\s+uses\\s+[A-Z][A-Za-z0-9_]*(?:\\s*,\\s*[A-Z][A-Za-z0-9_]*)*\\s*;?\\s*(?://.*)?$")

        private val LEGACY_TRAIT_USE_LINE =
            Regex("^\\s+use\\s+[A-Z][A-Za-z0-9_]*(?:\\s*,\\s*[A-Z][A-Za-z0-9_]*)*\\s*;?\\s*(?://.*)?$")

        private val IMPORT_USE_LINE =
            Regex("^use\\s+[A-Za-z_][A-Za-z0-9_]*(?:\\\\[A-Za-z_][A-Za-z0-9_]*)+(?:\\s+as\\s+[A-Za-z_][A-Za-z0-9_]*)?\\s*;?\\s*(?://.*)?$")

        private val PREPROCESSOR_DIRECTIVES = setOf(
            "include",
            "define",
            "undef",
            "if",
            "ifdef",
            "ifndef",
            "elif",
            "else",
            "endif",
            "warning",
            "error",
        )

        private val DOC_TYPE_TAGS = setOf(
            "param",
            "return",
            "var",
            "property",
            "property-read",
            "property-write",
            "method",
            "throws",
            "template",
            "extends",
            "implements",
        )

        private val STRICT_COMPARISON_OPERATORS = setOf("===", "!==")
        private val THREE_CHAR_OPERATORS = STRICT_COMPARISON_OPERATORS + setOf("..<")
        private val TWO_CHAR_OPERATORS = setOf("==", "!=", "<=", ">=", "&&", "||", "??", "=>", "+=", "-=", "++", "--", "..", "->", "::")
    }
}
