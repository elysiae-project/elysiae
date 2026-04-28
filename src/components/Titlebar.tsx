import { cva } from "class-variance-authority";
import { useGame } from "../hooks/useGame.ts";
import { Variants } from "../types";
import { closeApp } from "../util/AppFunctions.ts";
import { AnimatePresence } from "motion/react";
import MenuClose from "./MenuClose.tsx";

const titlebarStyles = cva(
	"h-15 min-w-full p-1 transition-all duration-250 overflow-y-hidden",
	{
		variants: {
			game: {
				[Variants.BH3]: "bg-bh3-titlebar rounded-t-xl",
				[Variants.HK4E]: "bg-hk4e-titlebar",
				[Variants.HKRPG]: "bg-hkrpg-titlebar titlebar-hkrpg-noise",
				[Variants.NAP]: "bg-nap-titlebar nap-dots-titlebar rounded-tl-xl",
			},
		},
	},
);

export default function Titlebar() {
	const { game } = useGame();
	return (
		<div class={titlebarStyles({ game: game })}>
			<div
				style={{ zIndex: 1001, color: "white" }}
				class="flex flex-row justify-between items-center px-5 py-1.5"
				data-tauri-drag-region>
				<h1 class="text-center text-[1.35rem]">Elysiae</h1>
				<AnimatePresence mode="wait" initial={false}>
					<MenuClose clickAction={closeApp} />
				</AnimatePresence>
			</div>
		</div>
	);
}
