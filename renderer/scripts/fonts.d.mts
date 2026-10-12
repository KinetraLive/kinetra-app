// Types for fonts.mjs, so renderer tests can import it under the renderer tsconfig.
export declare const SLUGS: string[];
export declare const WEIGHTS: number[];
export declare const FAMILY_WEIGHTS: Record<string, number[]>;
export declare function weightsOf(slug: string): number[];
export declare const FAMILY_ITALICS: Record<string, number[]>;
export declare function italicsOf(slug: string): number[];
export declare const SUBSETS: string[];
export declare function plan(modules?: string): { slug: string; css: string; files: { from: string; name: string; bytes: number }[] }[];
export declare function run(out?: string, dryRun?: boolean): number;
