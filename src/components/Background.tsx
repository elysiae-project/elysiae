import { AnimatePresence, motion } from "motion/react";
import { useApi } from "../hooks/useApi";
import { useGame } from "../hooks/useGame";
import { fadeInOut } from "../util/Animations";

function BackgroundMedia({ src, isVideo }: { src: string; isVideo: boolean }) {
  return isVideo ? (
    <motion.video
      class="background"
      {...(fadeInOut as any)}
      src={src}
      autoplay
      loop
      muted
      playsInline
    />
  ) : (
    <motion.img class="background" {...(fadeInOut as any)} src={src} alt="" />
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
