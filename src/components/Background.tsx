import { AnimatePresence, motion } from "motion/react";
import { useApi } from "../hooks/useApi";
import { useGame } from "../hooks/useGame";

const BackgroundImage = ({ src }: { src: string }) => {
	/* The extra scale is needed to prevent the overlay from
	 * showing the border it has for some reason. scale is very miniscule,
	 * I don't think it's noticable at all (1920x1080 -> 1928x1084)
	 */
	return (
		<motion.img
			class="background"
			initial={{ opacity: 0 }}
			animate={{ opacity: 1 }}
			exit={{ opacity: 0 }}
			transition={{ duration: 0.25, ease: "easeInOut" }}
			src={src}
			alt=""
		/>
	);
};

function BackgroundVideo({ src }: { src: string }) {
	return (
		<motion.video
			class="background"
			initial={{ opacity: 0 }}
			animate={{ opacity: 1 }}
			exit={{ opacity: 0 }}
			transition={{ duration: 0.25, ease: "easeInOut" }}
			src={src}
			autoplay
			loop
			muted
			playsInline
		/>
	);
}

export default function Background() {
	const { game } = useGame();
	const { graphics } = useApi();

	if (!graphics) return null;
	const { backgroundImage, backgroundVideo, backgroundVideoOverlay } =
		graphics[game];
	const isVideo = backgroundVideo !== "";

	return (
		<div class="absolute inset-0 overflow-hidden">
			<AnimatePresence mode="sync">
				{/* Game Background */}
				{isVideo ? (
					<BackgroundVideo key={`${game}-bg`} src={backgroundVideo} />
				) : (
					<BackgroundImage key={`${game}-bg`} src={backgroundImage} />
				)}

				{/* Game Overlay */}
				<BackgroundImage key={`${game}-overlay`} src={backgroundVideoOverlay} />
			</AnimatePresence>
		</div>
	);
}
