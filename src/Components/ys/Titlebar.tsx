import TitlebarButtons from "./TitlebarButtons"


export default function Titlebar() {
    return (
        <div data-tauri-drag-region class="bg-ys-titlebar min-w-full h-auto p-1 font-genshin">
            <div data-tauri-drag-region class="bg-transparent border border-[#505869] flex flex-row justify-between px-5 py-3">
                <h3 class="text-xl" data-tauri-drag-region>Yoohoo!</h3>
                <TitlebarButtons/>
            </div>
        </div>
    )
}
