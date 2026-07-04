package dev.doria.intellij.highlighting

import com.intellij.openapi.editor.colors.TextAttributesKey
import com.intellij.openapi.fileTypes.SyntaxHighlighter
import com.intellij.openapi.options.colors.AttributesDescriptor
import com.intellij.openapi.options.colors.ColorDescriptor
import com.intellij.openapi.options.colors.ColorSettingsPage
import dev.doria.intellij.DoriaIcons
import javax.swing.Icon

class DoriaColorSettingsPage : ColorSettingsPage {
    override fun getIcon(): Icon = DoriaIcons.FILE

    override fun getHighlighter(): SyntaxHighlighter = DoriaSyntaxHighlighter()

    override fun getDemoText(): String = """
        class Person
        {
            writable string ${'$'}name = "Andrew Masiye";

            function greet(): void
            {
                echo ${'$'}this->getGreetingMessage();
            }

            /**
             * Rename the person.
             *
             * @param string ${'$'}name The new name.
             */
            writable function rename(string ${'$'}name): void
            {
                ${'$'}this->name = ${'$'}name;
            }

            /**
             * Get the greeting message.
             *
             * @return string The greeting message.
             */
            internal function getGreetingMessage(): string
            {
                return "Hello, my name is {${'$'}this->name}";
            }
        }

        let writable ${'$'}person = new Person();
        echo "\\n---\\t---\\r---\\s";
        ${'$'}person->greet();
        Person::fromName("Lucy");
    """.trimIndent()

    override fun getAdditionalHighlightingTagToDescriptorMap(): Map<String, TextAttributesKey> = emptyMap()

    override fun getAttributeDescriptors(): Array<AttributesDescriptor> = ATTRIBUTES

    override fun getColorDescriptors(): Array<ColorDescriptor> = ColorDescriptor.EMPTY_ARRAY

    override fun getDisplayName(): String = "Doria"

    companion object {
        private val ATTRIBUTES = arrayOf(
            AttributesDescriptor("Keyword", DoriaSyntaxHighlighter.KEYWORD),
            AttributesDescriptor("Modifier", DoriaSyntaxHighlighter.MODIFIER),
            AttributesDescriptor("Primitive type", DoriaSyntaxHighlighter.PRIMITIVE_TYPE),
            AttributesDescriptor("Collection type", DoriaSyntaxHighlighter.COLLECTION_TYPE),
            AttributesDescriptor("Type name", DoriaSyntaxHighlighter.TYPE_NAME),
            AttributesDescriptor("Import use keyword", DoriaSyntaxHighlighter.IMPORT_USE_KEYWORD),
            AttributesDescriptor("Import path", DoriaSyntaxHighlighter.IMPORT_PATH),
            AttributesDescriptor("Import alias keyword", DoriaSyntaxHighlighter.IMPORT_ALIAS_KEYWORD),
            AttributesDescriptor("Import alias", DoriaSyntaxHighlighter.IMPORT_ALIAS),
            AttributesDescriptor("Trait uses keyword", DoriaSyntaxHighlighter.TRAIT_USES_KEYWORD),
            AttributesDescriptor("Trait name", DoriaSyntaxHighlighter.TRAIT_NAME),
            AttributesDescriptor("Function declaration", DoriaSyntaxHighlighter.FUNCTION_DECLARATION),
            AttributesDescriptor("Function call", DoriaSyntaxHighlighter.FUNCTION_CALL),
            AttributesDescriptor("Method call", DoriaSyntaxHighlighter.METHOD_CALL),
            AttributesDescriptor("Static method call", DoriaSyntaxHighlighter.STATIC_METHOD_CALL),
            AttributesDescriptor("Identifier", DoriaSyntaxHighlighter.IDENTIFIER),
            AttributesDescriptor("Variable", DoriaSyntaxHighlighter.VARIABLE),
            AttributesDescriptor("This", DoriaSyntaxHighlighter.THIS),
            AttributesDescriptor("Unused variable", DoriaSyntaxHighlighter.UNUSED_VARIABLE),
            AttributesDescriptor("Constant", DoriaSyntaxHighlighter.CONSTANT),
            AttributesDescriptor("Number", DoriaSyntaxHighlighter.NUMBER),
            AttributesDescriptor("String", DoriaSyntaxHighlighter.STRING),
            AttributesDescriptor("Escape sequence", DoriaSyntaxHighlighter.ESCAPE_SEQUENCE),
            AttributesDescriptor("Comment", DoriaSyntaxHighlighter.COMMENT),
            AttributesDescriptor("Doc comment", DoriaSyntaxHighlighter.DOC_COMMENT),
            AttributesDescriptor("Doc comment tag", DoriaSyntaxHighlighter.DOC_COMMENT_TAG),
            AttributesDescriptor("Invalid or rejected syntax", DoriaSyntaxHighlighter.INVALID),
            AttributesDescriptor("Operator", DoriaSyntaxHighlighter.OPERATOR),
            AttributesDescriptor("Braces", DoriaSyntaxHighlighter.BRACE),
            AttributesDescriptor("Brackets", DoriaSyntaxHighlighter.BRACKET),
            AttributesDescriptor("Parentheses", DoriaSyntaxHighlighter.PAREN),
            AttributesDescriptor("Punctuation", DoriaSyntaxHighlighter.PUNCTUATION),
            AttributesDescriptor("Bad character", DoriaSyntaxHighlighter.BAD_CHARACTER),
        )
    }
}
