package dev.doria.intellij.settings

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.components.PersistentStateComponent
import com.intellij.openapi.components.State
import com.intellij.openapi.components.Storage

@State(name = "DoriaSettings", storages = [Storage("doria.xml")])
class DoriaSettings : PersistentStateComponent<DoriaSettings.State> {
    data class State(
        var languageServerPath: String = "",
    )

    private var state = State()

    override fun getState(): State = state

    override fun loadState(state: State) {
        this.state = state
    }

    companion object {
        fun getInstance(): DoriaSettings =
            ApplicationManager.getApplication().getService(DoriaSettings::class.java)
    }
}
