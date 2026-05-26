import { AnimatePresence, motion } from "motion/react";
import { useRef } from "preact/hooks";
import { useApi } from "../../hooks/useApi";
import { useGame } from "../../hooks/useGame";

const BackgroundMedia = ({
	src,
	isVideo,
}: {
	src: string | null;
	isVideo: boolean;
}) => {
	const videoRef = useRef<HTMLVideoElement>(null);
	// const localSrc = useState<string | null>(null);

	if(!src) return null;

	return isVideo ? (
		<motion.video
			ref={videoRef}
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

	return (
		<div class="absolute inset-0 overflow-hidden">
			<AnimatePresence mode="wait">
				<BackgroundMedia
					key={`${game}-bg`}
					src={isVideo ? backgroundVideo : backgroundImage}
					isVideo={isVideo}
				/>
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
