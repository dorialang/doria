package dev.doria.intellij.highlighting

import com.intellij.lang.annotation.AnnotationHolder
import com.intellij.lang.annotation.Annotator
import com.intellij.lang.annotation.HighlightSeverity
import com.intellij.openapi.util.TextRange
import com.intellij.psi.PsiElement
import com.intellij.psi.PsiFile
import com.intellij.psi.tree.IElementType
import dev.doria.intellij.DoriaFileType

class DoriaVariableUsageAnnotator : Annotator {
    override fun annotate(element: PsiElement, holder: AnnotationHolder) {
        val file = element as? PsiFile ?: return
        if (file.fileType != DoriaFileType.INSTANCE) {
            return
        }

        val occurrences = collectVariableOccurrences(file.text)
        if (occurrences.isEmpty()) {
            return
        }

        val referencedNames = occurrences
            .filterNot { it.isDeclaration }
            .mapTo(mutableSetOf()) { it.name }

        for (occurrence in occurrences) {
            if (occurrence.name in referencedNames) {
                continue
            }

            holder.newSilentAnnotation(HighlightSeverity.INFORMATION)
                .range(occurrence.range)
                .textAttributes(DoriaSyntaxHighlighter.UNUSED_VARIABLE)
                .create()
        }
    }

    private fun collectVariableOccurrences(text: String): List<VariableOccurrence> {
        val tokens = tokenize(text)
        return tokens.mapIndexedNotNull { index, token ->
            if (token.type != DoriaTokenTypes.VARIABLE || token.state == DoriaLexer.MODE_DOC_COMMENT) {
                return@mapIndexedNotNull null
            }

            VariableOccurrence(
                name = token.text,
                range = TextRange(token.startOffset, token.endOffset),
                isDeclaration = isDeclaration(tokens, index),
            )
        }
    }

    private fun tokenize(text: String): List<DoriaToken> {
        val lexer = DoriaLexer()
        lexer.start(text)

        val tokens = mutableListOf<DoriaToken>()
        while (lexer.tokenType != null) {
            val type = lexer.tokenType ?: break
            if (type != DoriaTokenTypes.WHITE_SPACE && type != DoriaTokenTypes.COMMENT) {
                tokens += DoriaToken(
                    type = type,
                    text = text.substring(lexer.tokenStart, lexer.tokenEnd),
                    startOffset = lexer.tokenStart,
                    endOffset = lexer.tokenEnd,
                    state = lexer.state,
                )
            }
            lexer.advance()
        }

        return tokens
    }

    private fun isDeclaration(tokens: List<DoriaToken>, index: Int): Boolean {
        val previous = tokens.getOrNull(index - 1) ?: return false
        if (previous.text == "let" || previous.text == "as") {
            return true
        }

        if (previous.type in DECLARATION_PREFIX_TYPES) {
            return true
        }

        if (previous.text == ">") {
            return isAfterGenericType(tokens, index - 1)
        }

        return false
    }

    private fun isAfterGenericType(tokens: List<DoriaToken>, genericEndIndex: Int): Boolean {
        var depth = 0
        for (index in genericEndIndex downTo 0) {
            when (tokens[index].text) {
                ">" -> depth++
                "<" -> {
                    depth--
                    if (depth == 0) {
                        val typeName = tokens.getOrNull(index - 1)
                        return typeName?.type in DECLARATION_PREFIX_TYPES
                    }
                }
            }
        }
        return false
    }

    private data class DoriaToken(
        val type: IElementType,
        val text: String,
        val startOffset: Int,
        val endOffset: Int,
        val state: Int,
    )

    private data class VariableOccurrence(
        val name: String,
        val range: TextRange,
        val isDeclaration: Boolean,
    )

    companion object {
        private val DECLARATION_PREFIX_TYPES = setOf(
            DoriaTokenTypes.MODIFIER,
            DoriaTokenTypes.PRIMITIVE_TYPE,
            DoriaTokenTypes.COLLECTION_TYPE,
            DoriaTokenTypes.TYPE_NAME,
        )
    }
}
