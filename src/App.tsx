import "./App.css";
import { GameContext, useGame, Variants } from "./util/selectedGame.ts";
import Titlebar from "./Components/Titlebar.tsx";
import { cva } from "class-variance-authority";

const theme = cva("h-full w-full px-3 py-4", {
	variants: {
		intent: {
			[Variants.BH]: "bg-bh-bg font-hsr-hi3 rounded-b-xl text-white",
			[Variants.YS]: "bg-ys-bg font-genshin text-black",
			[Variants.SR]:
				"bg-sr-bg font-hsr-hi3 rounded-b-xs border border-[#393939] text-black",
			[Variants.NAP]:
				"bg-nap-bg font-zzz rounded-br-xl border-b-2 border-r-2 border-l-2 border-nap-border text-white nap-dots",
		},
	},
});

function Background() {}

export default function App() {
	return (
		<GameContext.Provider value={useGame()}>
			<div class="flex h-screen w-screen flex-col gap-0 text-white">
				<Titlebar />
				<div class={theme({ intent: useGame() })}>
					<div class="flex h-full w-full flex-col items-center justify-center text-center">
						<h1 class="text-8xl">It's Taurin'</h1>
						<h2 class="text-6xl">Time</h2>
					</div>
				</div>
			</div>
		</GameContext.Provider>
	);
}
