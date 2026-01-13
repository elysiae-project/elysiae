<script lang="ts">
    import gsap from "gsap";
    import "../app.css";
    import { X, Scan, Minus } from "@lucide/svelte";
    import { exit } from "@tauri-apps/plugin-process";
    import { getCurrentWindow } from "@tauri-apps/api/window";

    let { children } = $props();
    const appWindow = getCurrentWindow();

    function minimize() {
        appWindow.minimize();
    }

    function maximize() {
        appWindow.toggleMaximize();
    }

    async function close() {
        await exit();
    }

</script>

<div class="w-screen h-screen flex flex-col dark:text-white text-black">
    <div data-tauri-drag-region class="w-full h-10 bg-neutral-600 flex flex-row justify-between items-center px-3 py-1">
        <div class="flex flex-row">
            <h1>Yoohoo!</h1>
        </div>
        <div class="h-full flex flex-row justify-end items-center gap-2.5">
            <Minus class="minus p-1" onclick={minimize}/>
            <Scan class="maximize p-1" 
                  onclick={(e) => {e.currentTarget.blur(); maximize()}} 
                onmousedown={(e) => {
                    e.currentTarget.blur(); 
                    gsap.fromTo(".maximize", { 
                        scaleX: 0.65, 
                        scaleY: 0.65, 
                        duration: 0.35 
                    },
                    { 
                        scaleX: 1.0, 
                        scaleY: 1.0 , 
                        duration: 0.35 
                    });
                }}/>
            <X class="close p-1" onclick={close} />
        </div>
    </div>
    <div class="px-5 py-3 bg-neutral-800 h-full w-full">
        {@render children?.()}
    </div>
</div>
