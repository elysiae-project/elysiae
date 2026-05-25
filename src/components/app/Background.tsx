import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef, useState } from "preact/hooks";
import { useApi } from "../../hooks/useApi";
import { useGame } from "../../hooks/useGame";
import { fadeInOut } from "../../util/Animations";

function BackgroundMedia({
	src,
	isVideo,
}: {
	src: string | null;
	isVideo: boolean;
}) {
	const videoRef = useRef<HTMLVideoElement>(null);
	const localSrc = useState<string | null>(null);



	useEffect(() => {
		if (isVideo && videoRef.current) {
			videoRef.current.play().catch(() => {});
		}
	}, [isVideo, src]);

	if(!localSrc) {
		return <div></div>
	}

	return isVideo ? (
		<motion.video
			ref={videoRef}
			class="background"
			// biome-ignore lint/suspicious/noExplicitAny: Stops a stupid type error
			{...(fadeInOut as any)}
			src={src}
			autoplay
			loop
			muted
			playsInline
		/>
	) : (
		// biome-ignore lint/suspicious/noExplicitAny: Stops a stupid type error
		<motion.img class="background" {...(fadeInOut as any)} src={src} alt="" />
	);
}

export default function Background() {
	const { game } = useGame();
	const { graphics, backgrounds } = useApi();

	if (!graphics || !backgrounds) return null;
	const { backgroundImage, backgroundVideo } = backgrounds[game];
	const { backgroundVideoOverlay } = graphics[game];

	const isVideo = backgroundVideo !== null && backgroundVideo !== "";

	return (
		<div class="absolute inset-0 overflow-hidden">
			<AnimatePresence mode="popLayout">
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
}
