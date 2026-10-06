# 18. Point clouds

Arkestra's 3D tracks are its showpiece: a live picture lifted into a cloud of points by its
brightness, then pushed around by noise, a spring particle sim and force fields, with bloom.
tripslop already has a mesh renderer with MSAA, lighting and a camera (`meshes.rs`), so a point
cloud is one more thing it can draw.

## Plan

A **Point cloud** effect (category Space). Like the shape projector, it takes the layer as its
input and draws a 3D scene.

* **Points from the picture:** a grid of up to 512×288 points samples the input. Each point's
  colour is the pixel's colour, and its depth is the pixel's luma (or one channel), times
  *relief*. *Black clip* drops dark points, so black backgrounds fall away.
* **Drawing:** instanced quads facing the camera, as **dots** (round, soft), **squares** or
  **lines** (connecting neighbours along rows, like a scan-line terrain). *Size*, *size by luma*,
  and additive or opaque drawing.
* **Camera:** orbit with yaw, pitch, distance and FOV, plus beat-synced spin like the shape
  projector. Drag on the monitor to orbit while the card is selected.
* **Motion**, in order, all on the GPU in a compute pass over a storage buffer of positions and
  velocities:
  1. **Noise**: curl noise added to the rest positions (amplitude, scale, speed, octaves).
  2. **Particle sim** (optional): each point springs back to its rest position (*spring*,
     *damping*), pushed by up to 4 **forces**: radial out/in, vortex, directional, noise, and a
     **pulse**, a radial burst fired by a trigger parameter. Map the trigger to the kick and the
     cloud explodes on every beat.
  3. **Flow**: drift along an axis with wrap-around, for endless fly-throughs.
* **Bloom** (a small mip-chain bloom at the end of the mesh pass), off by default.
* **Models too:** with *source* set to *Model*, the points come from a model's vertices (and
  barycentric samples on large faces) instead of the picture, so the bunny can dissolve into
  dust.

## Order

Static cloud from the picture, then drawing modes and camera, then noise, then the particle
sim and forces, then model sources, then bloom. Each step is shippable.

## Scripts and tests

`scripts/point-cloud.tripslop` snapshots the demo's jellyfish as a cloud from three camera
angles, and after 8 beats of a pulse mapped to a sequencer. Unit tests cover the grid sizing and
the spring integration (a displaced point settles back within tolerance).

## Done when

* The jellyfish clip, as a point cloud, bursts apart on the kick and pulls itself back together.
