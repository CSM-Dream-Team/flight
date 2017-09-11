#version 410
 
layout(std140) uniform transform {
    mat4 model;
    mat4 view;
    mat4 proj;
    float xoffset;
};

in vec3 a_pos;
out vec3 v_pos;

#ifdef NORM
in vec3 a_norm;
out vec3 v_norm;
#endif

#ifdef TEX
in vec2 a_tex;
out vec2 v_tex;
#endif

#ifdef COLOR
in vec3 a_color;
out vec3 v_color;
#endif

void main() {
    vec4 p = model * vec4(a_pos, 1);
    v_pos = p.xyz;

    #ifdef NORM
    v_norm = (model * vec4(a_nor, 0)).xyz;
    #endif

    #ifdef TEX
    v_tex = a_tex;
    v_tex.y = 1 - v_tex.y;
    #endif

    #ifdef COLOR
    v_color = a_color;
    #endif

    vec4 c = proj * view * p;
    // Fake an opengl viewport
    // TODO: Submit a PR to GFX
    c.x /= 2 * c.w;
    c.x += xoffset;
    c.x *= c.w;
    gl_Position = c;
}