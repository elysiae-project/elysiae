import { useEffect, useRef } from "preact/hooks";
import { useApi } from "../../hooks/useApi";
import { useBackground } from "../../hooks/useBackground";
import { useGame } from "../../hooks/useGame";

const BackgroundVideo = ({ src }: { src: string | null }) => {
	const ref = useRef<HTMLVideoElement>(null);
	const pendingSrc = useRef<string | null>(null);
	const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

	useEffect(() => {
		const el = ref.current;
		if (!el) return;

		if (!src) {
			el.removeAttribute("src");
			el.load();
			return;
		}

		if (!el.src || el.readyState === 0) {
			el.src = src;
			el.load();
			return;
		}

		el.pause();
		el.removeAttribute("src");
		el.load();

		pendingSrc.current = src;
		if (timerRef.current) clearTimeout(timerRef.current);
		timerRef.current = setTimeout(() => {
			if (pendingSrc.current && ref.current) {
				ref.current.src = pendingSrc.current;
				ref.current.load();
				pendingSrc.current = null;
			}
		}, 50);

		return () => {
			if (timerRef.current) clearTimeout(timerRef.current);
		};
	}, [src]);

	return <video ref={ref} class="background" autoPlay loop muted playsInline />;
};

const BackgroundMedia = ({
	src,
	isVideo,
}: {
	src: string | null;
	isVideo: boolean;
}) => {
	if (!src) return null;

	return isVideo ? (
		<BackgroundVideo src={src} />
	) : (
		<img class="background" src={src} alt="" />
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
