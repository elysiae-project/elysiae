import { useApi } from "../../hooks/useApi";
import { useGame } from "../../hooks/useGame";
import { motion } from "motion/react";

export default function Sidebar() {
	const { game, setGame } = useGame();
	const { branding } = useApi();

	return (
		<motion.div
			style={{ translateX: "-20%" }}
			class="absolute h-auto max-h-100 overflow-y-scroll rounded-xl self-center bg-black/80 bottom-0 right-0 top-0 z-20 flex flex-col justify-center items-center gap-y-4 p-4">
			{branding &&
				Object.entries(branding).map(([key, data]) => (
					<button
						key={key}
						class="relative h-12 w-12 cursor-pointer rounded-lg border-white transition-transform duration-150 hover:scale-110 active:scale-90"
						style={{
							borderWidth: game === +key ? "0.125rem" : "",
						}}
						onClick={() => setGame(+key)}>
						<img
							class={`absolute inset-0 rounded-lg ${false ? "monochromatic" : ""} transition`}
							src={data.icon}
							alt=""
						/>
					</button>
				))}
		</motion.div>
	);
}
