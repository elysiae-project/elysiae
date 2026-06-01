import { AnimatePresence, motion } from "motion/react";
import { useApi } from "../../hooks/useApi";
import { useGame } from "../../hooks/useGame";

const BackgroundMedia = ({
	src,
	isVideo,
}: {
	src: string | null;
	isVideo: boolean;
}) => {
	if (!src) return null;

	return isVideo ? (
		<motion.video
			class="background"
			initial={{ opacity: 0 }}
			animate={{ opacity: 1 }}
			exit={{ opacity: 0 }}
			transition={{ duration: 0.25, ease: "easeInOut" }}
			src={src as string}
			autoplay
			loop
			muted
			playsInline
		/>
	) : (
		<motion.img
			class="background"
			initial={{ opacity: 0 }}
			animate={{ opacity: 1 }}
			exit={{ opacity: 0 }}
			transition={{ duration: 0.25, ease: "easeInOut" }}
			src={src as string}
			alt=""
		/>
	);
};

export const Background = () => {
	const { game } = useGame();
	const { graphics, backgrounds } = useApi();

	if (!graphics || !backgrounds) return null;
	const { backgroundImage, backgroundVideo } = backgrounds[game];
	const { backgroundVideoOverlay } = graphics[game];

	const isVideo = backgroundVideo !== null && backgroundVideo !== "";

	console.log(backgroundVideo);
	console.log(backgroundImage);

	return (
		<div class="absolute inset-0 overflow-hidden">
			<AnimatePresence mode="wait">
				<BackgroundMedia
					key={`${game}-bg`}
					src={isVideo ? backgroundVideo : backgroundImage}
					isVideo={isVideo}
				/>
			</AnimatePresence>
			<AnimatePresence mode="wait">
				<BackgroundMedia
					key={`${game}-overlay`}
					src={backgroundVideoOverlay}
					isVideo={false}
				/>
			</AnimatePresence>
		</div>
	);
};

export default Background;
