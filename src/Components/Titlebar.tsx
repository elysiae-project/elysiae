import { getCurrentWindow } from '@tauri-apps/api/window';
import { cva } from "class-variance-authority";
import { X, Maximize, Minus } from 'lucide-preact';
import { useGame } from "../util/selectedGame.ts";

const appWindow = getCurrentWindow();

const titlebar = cva("flex flex-row justify-between w-full h-auto px-3 py-1.5 rounded-t-xl", {
    variants: {
        intent: {
            hi3: "bg-amber-300 font-hsr-hi3",
            genshin: "bg-orange-400 font-genshin text-white",
            hsr: "bg-pink-500 font-hsr-hi3",
            zzz: "bg-green-400 font-zzz"
        }
    }
});

const closeWindow = () => {
    appWindow.close();
}

const toggleMaximize = () => {
    appWindow.toggleMaximize();
}

const minimize = () => {
    appWindow.minimize();
}

export default function Titlebar() {
    return(
        <>
            <div data-tauri-drag-region class={titlebar({intent: useGame()})}>
                <p data-tauri-drag-region>Yoohoo!</p>
                <div class="flex flex-row-reverse gap-2 ">
                    <button onClick={() => closeWindow()}><X/></button>
                    <button onClick={() => toggleMaximize()}><Maximize/></button>
                    <button onClick={() => minimize()}><Minus/></button>
                </div>
            </div>
        </>
    )
}