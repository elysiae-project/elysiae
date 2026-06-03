import { useEffect, useRef } from "preact/hooks";
import { useApi } from "../../hooks/useApi";
import { useBackground } from "../../hooks/useBackground";
import { useGame } from "../../hooks/useGame";

const BackgroundMedia = ({
	src,
	isVideo,
}: {
	src: string | null;
	isVideo: boolean;
}) => {
	console.log(`Source: ${src}, is video? ${isVideo}`);
	const video = useRef<HTMLVideoElement>(null);
	const image = useRef<HTMLImageElement>(null);

	if (!src && (!video || !image)) return null;
	useEffect(() => {
		if (isVideo) {
			video.current?.load();
		}
	}, [src, isVideo]);

	return isVideo ? (
		<video
			ref={video}
			class="background"
			src={src as string}
			autoplay
			loop
			muted
			playsInline
		/>
	) : (
		<img class="background" src={src as string} alt="" />
	);
};

export const Background = () => {
	const { game } = useGame();
	const { graphics } = useApi();
	const { backgroundBlob, backgroundPath } = useBackground();

	if (!backgroundBlob || !backgroundPath || !graphics) return null;
	const { backgroundVideoOverlay } = graphics[game];

	const isVideo = backgroundPath.endsWith(".mp4");

	return (
		<div class="absolute inset-0 overflow-hidden">
			<BackgroundMedia
				key={`${game}-bg`}
				src={backgroundBlob}
				isVideo={isVideo}
			/>
			<BackgroundMedia
				key={`${game}-overlay`}
				src={backgroundVideoOverlay}
				isVideo={false}
			/>
		</div>
	);
};

export default Background;
