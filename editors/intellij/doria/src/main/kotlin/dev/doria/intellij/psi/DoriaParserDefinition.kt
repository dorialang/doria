package dev.doria.intellij.psi

import com.intellij.extapi.psi.ASTWrapperPsiElement
import com.intellij.extapi.psi.PsiFileBase
import com.intellij.lang.ASTNode
import com.intellij.lang.ParserDefinition
import com.intellij.lang.PsiParser
import com.intellij.lexer.Lexer
import com.intellij.openapi.project.Project
import com.intellij.psi.FileViewProvider
import com.intellij.psi.PsiElement
import com.intellij.psi.PsiFile
import com.intellij.psi.TokenType
import com.intellij.psi.tree.IFileElementType
import com.intellij.psi.tree.TokenSet
import dev.doria.intellij.DoriaFileType
import dev.doria.intellij.DoriaLanguage
import dev.doria.intellij.highlighting.DoriaLexer
import dev.doria.intellij.highlighting.DoriaTokenTypes

class DoriaParserDefinition : ParserDefinition {
    override fun createLexer(project: Project?): Lexer = DoriaLexer()

    override fun createParser(project: Project?): PsiParser = PsiParser { root, builder ->
        val file = builder.mark()
        while (!builder.eof()) {
            builder.advanceLexer()
        }
        file.done(root)
        builder.treeBuilt
    }

    override fun getFileNodeType(): IFileElementType = FILE

    override fun getWhitespaceTokens(): TokenSet = TokenSet.create(TokenType.WHITE_SPACE)

    override fun getCommentTokens(): TokenSet = TokenSet.create(
        DoriaTokenTypes.COMMENT,
        DoriaTokenTypes.DOC_COMMENT,
        DoriaTokenTypes.DOC_COMMENT_TAG,
    )

    override fun getStringLiteralElements(): TokenSet = TokenSet.create(
        DoriaTokenTypes.STRING,
        DoriaTokenTypes.ESCAPE_SEQUENCE,
    )

    override fun createElement(node: ASTNode): PsiElement = ASTWrapperPsiElement(node)

    override fun createFile(viewProvider: FileViewProvider): PsiFile = DoriaPsiFile(viewProvider)

    override fun spaceExistenceTypeBetweenTokens(
        left: ASTNode?,
        right: ASTNode?,
    ): ParserDefinition.SpaceRequirements = ParserDefinition.SpaceRequirements.MAY

    companion object {
        val FILE = IFileElementType(DoriaLanguage)
    }
}

class DoriaPsiFile(viewProvider: FileViewProvider) : PsiFileBase(viewProvider, DoriaLanguage) {
    override fun getFileType(): DoriaFileType = DoriaFileType.INSTANCE

    override fun toString(): String = "Doria File"
}
