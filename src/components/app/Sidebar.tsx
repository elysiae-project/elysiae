import { motion } from "motion/react";
import { useApi } from "../../hooks/useApi";
import { useGame } from "../../hooks/useGame";

export const Sidebar = () => {
	const { game, setGame } = useGame();
	const { branding } = useApi();

	return (
		<motion.div
			style={{ translateX: "-20%" }}
			class="absolute top-0 right-0 bottom-0 z-20 flex h-auto max-h-100 flex-col items-center justify-center gap-y-4 self-center overflow-y-scroll rounded-xl bg-black/80 p-4"
		>
			{branding &&
				Object.entries(branding).map(([key, data]) => (
					<button
						type="button"
						key={key}
						class="relative h-12 w-12 cursor-pointer rounded-lg border-white transition-transform duration-150 hover:scale-110 active:scale-90"
						style={{
							borderWidth: game === +key ? "0.125rem" : "",
						}}
						onClick={() => setGame(+key)}
					>
						<img
							class={`absolute inset-0 rounded-lg transition`}
							src={data.icon}
							alt=""
						/>
					</button>
				))}
		</motion.div>
	);
}

export default Sidebar;
