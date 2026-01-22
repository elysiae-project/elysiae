import "./App.css";
import { GameContext, useGame } from "./hooks/useGame.ts";
import Titlebar from "./components/Titlebar.tsx";
import { cva } from "class-variance-authority";
import { Variants } from "./types";
import { useApi } from "./hooks/useApi.ts";

const theme = cva("h-full w-full overflow-hidden", {
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
	const game = useGame();
	const api = useApi();

	return (
		<GameContext.Provider value={game}>
			<div class="flex h-screen w-screen flex-col gap-0 text-white">
				<Titlebar />

				<div class={theme({ intent: game })}>
					{api ? (
						<div class="relative h-full w-full">
							<video
								class="absolute inset-0 h-full w-full object-cover"
								src={api[game].backgroundVideo}
								autoplay
								loop
								muted
							></video>
							<img
								class="absolute inset-0 h-full w-full object-cover"
								src={api[game].backgroundVideoOverlay}
								alt=""
							/>
							<div class="absolute inset-0 z-10 flex flex-col items-center justify-center text-center">
								{/* Page content */}
							</div>
						</div>
					) : (
						<div class="flex h-full w-full flex-col items-center justify-center text-center">
							<h2 class="text-6xl">Loading...</h2>
						</div>
					)}
				</div>
			</div>
		</GameContext.Provider>
	);
}
