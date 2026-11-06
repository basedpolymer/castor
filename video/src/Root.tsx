import React from "react";
import { Composition } from "remotion";
import { FPS, Promo, TOTAL } from "./Promo";
import { loadFonts } from "./theme";

loadFonts();

export const Root: React.FC = () => (
  <>
    {(["en", "fr"] as const).map((lang) => (
      <React.Fragment key={lang}>
        <Composition id={`promo-${lang}`} component={Promo} durationInFrames={TOTAL} fps={FPS} width={1920} height={1080} defaultProps={{ lang }} />
        <Composition id={`promo-${lang}-vertical`} component={Promo} durationInFrames={TOTAL} fps={FPS} width={1080} height={1920} defaultProps={{ lang }} />
      </React.Fragment>
    ))}
  </>
);
