import { cva } from "class-variance-authority";
import { useGame } from "../hooks/useGame.ts";
import { Variants } from "../types";
import { closeApp } from "../util/AppFunctions.ts";
import { motion, AnimatePresence } from "motion/react";
import MenuClose from "./MenuClose.tsx";
import { fadeInOut } from "../util/Animations.ts";

const titlebarStyles = cva(
	"h-15 min-w-full p-1 transition-all duration-250 overflow-y-hidden",
	{
		variants: {
			game: {
				// Declare fonts because the titlebar is declared outside the actual app content. Can be easily fixed if I wasn't lazy
				[Variants.BH3]: "bg-bh-titlebar font-bh-sr rounded-t-xl",
				[Variants.HK4E]: "bg-ys-titlebar font-ys",
				[Variants.HKRPG]: "bg-sr-titlebar titlebar-sr-noise font-bh-sr",
				[Variants.NAP]:
					"bg-nap-titlebar nap-dots-titlebar font-nap rounded-tl-xl",
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
				data-tauri-drag-region
			>
				<AnimatePresence mode="wait" initial={false}>
					<motion.h1
						{...fadeInOut as any}
						key={`${game}-appTitle`}
						class="text-center text-[1.35rem]"
						data-tauri-drag-region
					>
						Elysiae
					</motion.h1>
				</AnimatePresence>
				<AnimatePresence mode="wait" initial={false}>
					<MenuClose clickAction={closeApp} />
				</AnimatePresence>
			</div>
		</div>
	);
}
