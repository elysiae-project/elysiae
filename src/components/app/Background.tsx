import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef, useState } from "preact/hooks";
import { useApi } from "../../hooks/useApi";
import { useBackground } from "../../hooks/useBackground";
import { useGame } from "../../hooks/useGame";

const BackgroundVideo = ({ src }: { src: string | null }) => {
	const ref = useRef<HTMLVideoElement>(null);
	const [visible, setVisible] = useState(false);

	useEffect(() => {
		setVisible(false);
	}, [src]);

	useEffect(() => {
		const el = ref.current;
		if (!el || !src) return;

		const onCanPlay = () => {
			el.play().catch(() => {});
			setVisible(true);
		};

		el.addEventListener("canplay", onCanPlay);
		el.src = src;
		el.load();

		return () => {
			el.removeEventListener("canplay", onCanPlay);
			el.pause();
			el.removeAttribute("src");
			el.load();
		};
	}, [src]);

	return (
		<motion.div
			class="absolute inset-0"
			animate={{ opacity: visible ? 1 : 0 }}
			transition={{ duration: 0.4 }}
		>
			<video
				ref={ref}
				class="background"
				autoPlay
				muted
				playsInline
				loop
			/>
		</motion.div>
	);
};

const BackgroundMedia = ({
	src,
	isVideo,
}: {
	src: string | null;
	isVideo: boolean;
}) => {
	if (!src) return null;

	return (
		<AnimatePresence mode="wait" initial={false}>
			{isVideo ? (
				<BackgroundVideo key={src} src={src} />
			) : (
				<motion.img
					key={src}
					src={src}
					class="background"
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
					exit={{ opacity: 0 }}
					transition={{ duration: 0.4 }}
				/>
			)}
		</AnimatePresence>
	);
};

export const Background = () => {
	const { game } = useGame();
	const { graphics } = useApi();
	const { backgroundSrc, backgroundIsVideo } = useBackground();

	if (!backgroundSrc || !graphics) return null;
	const { backgroundVideoOverlay } = graphics[game];

	return (
		<div class="absolute inset-0 overflow-hidden">
			<BackgroundMedia src={backgroundSrc} isVideo={backgroundIsVideo} />
			<BackgroundMedia src={backgroundVideoOverlay} isVideo={false} />
		</div>
	);
};

export default Background;