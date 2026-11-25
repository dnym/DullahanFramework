#version 330 core
in vec2 v_uv;

uniform sampler2D u_scene_tex;
uniform float u_exposure;
uniform float u_contrast;
uniform float u_saturation;
uniform float u_vignette;
uniform float u_bloom_strength;
uniform float u_bloom_threshold;
uniform float u_chroma;
uniform float u_time;
uniform vec4 u_tint;

out vec4 o_color;

vec3 extract_neon(vec3 c, float thr) {
    float l = dot(c, vec3(0.2126, 0.7152, 0.0722));
    float mx = max(max(c.r, c.g), c.b);
    float sat = mx - min(min(c.r, c.g), c.b);
    float energy = max(l, mx * 0.90 + sat * 0.18);
    return c * smoothstep(thr, thr + 0.22, energy);
}

void main() {
    vec2 d = v_uv - 0.5;
    float r2 = dot(d, d);

    // Cinematic radial chromatic aberration (subtle at cruise, crisp punch on boost/impact)
    vec3 col;
    float ca = u_chroma * (0.0018 + r2 * 0.011);
    if (ca > 0.0001) {
        vec2 dir = normalize(d + vec2(1e-5));
        col.r = texture(u_scene_tex, v_uv + dir * ca).r;
        col.g = texture(u_scene_tex, v_uv).g;
        col.b = texture(u_scene_tex, v_uv - dir * ca).b;
    } else {
        col = texture(u_scene_tex, v_uv).rgb;
    }

    // Lush 2-ring synthwave neon bloom halo
    if (u_bloom_strength > 0.001) {
        vec2 texel = 1.0 / vec2(textureSize(u_scene_tex, 0));
        vec3 b_near = vec3(0.0);
        vec3 b_wide = vec3(0.0);
        for (int y = -1; y <= 1; ++y) {
            for (int x = -1; x <= 1; ++x) {
                vec2 o1 = vec2(x, y) * texel * 2.8;
                vec2 o2 = vec2(x, y) * texel * 7.5;
                b_near += extract_neon(texture(u_scene_tex, v_uv + o1).rgb, u_bloom_threshold);
                b_wide += extract_neon(texture(u_scene_tex, v_uv + o2).rgb, u_bloom_threshold);
            }
        }
        col += ((b_near * 0.62 + b_wide * 0.38) / 9.0) * u_bloom_strength;
    }

    // Exposure, contrast, saturation & tint flash
    col *= u_exposure;
    col = (col - 0.5) * u_contrast + 0.5;
    float luma = dot(col, vec3(0.2126, 0.7152, 0.0722));
    col = mix(vec3(luma), col, u_saturation);
    col *= u_tint.rgb;

    // Ultra-subtle synthwave CRT shimmer + twilight vignette
    float scan = 0.988 + 0.012 * sin(gl_FragCoord.y * 1.4 + u_time * 4.0);
    float vig = clamp(1.0 - r2 * (u_vignette * 2.4), 0.25, 1.0);
    o_color = vec4(clamp(col * scan * vig, 0.0, 1.0), 1.0);
}
