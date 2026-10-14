# Shell meshing literature survey for FieldCraft: quad and quad-dominant meshes of curved B-rep faces

Compiled 2026-10-13. Scope: all-quad and quad-dominant surface meshes of the curved faces (planes, cylinders,
cones, spheres, tori, NURBS) of SolidCraft parts, meshed in each face's (u,v) parameter space with metric
correction and conforming across shared B-rep edges, for shell analysis (MITC4-class shells now,
continuum shells on swept hexes in M8). This extends `docs/meshing/literature.md` §B7, which compared
paving, Q-Morph and Blossom-Quad for **planar** faces only and chose CDT → greedy recombination →
Catmull–Clark split (`fieldcraft-mesh::quad`).

Same format as `docs/meshing/literature.md`: per paper, citation, DOI/URL, method, problem solved,
robustness/limits, complexity, code + licence, verdict. Part P is the opinionated plan.

## How this was verified

- **DOIs.** Every DOI below returned `responseCode 1` from `doi.org/api/handles/<doi>` on 2026-10-13, and
  its title, authors, year, venue, volume and pages were checked against Crossref metadata. ACM TOG article numbers are omitted because Crossref does not carry them. Items without a
  DOI (Meshing Roundtable papers before 2007, preprints) are cited by venue with a URL that returned HTTP 200
  on the same day, or are listed under "Unverified items".
- **Read depth** (stated per entry as *Read:*). *full* = I read the paper (PDF). *abstract* = abstract and
  metadata only. *metadata* = title/venue only; the method line is then the paper's well-established
  content as cited by later papers, and I give no numbers or section numbers for it. Numbers quoted are from
  papers marked *full*.
- **Licences** come from GitHub licence metadata and LICENSE/README files only. No source code was opened.
  Legend as in `literature.md`: **READ OK** (MIT, BSD, Apache-2.0, MPL-2.0), **FORBIDDEN** (GPL, LGPL,
  AGPL), **NO LICENCE** (all rights reserved; treat as forbidden). "BSD-3+" means BSD-3-Clause plus the
  "enhancements" paragraph (still permissive).
- **Commercial-product rule.** Several reference papers validate on ABC (Onshape) or MAMBO (STEP files whose
  headers name commercial tools); see `literature.md` Part C. We cite their published numbers as evidence of
  the method's state, never use their data.

### Corrections found in passing
1. `docs/meshing/literature.md` B7.2 cites *A new scheme for the generation of a graded quadrilateral mesh*
   (C&S 52(5), 1994) as "D. T. Lee, S. H. Lo". Crossref gives **C. K. Lee** and S. H. Lo (C. K. Lee is the
   same author as S2.4 below). Fix the citation there.
2. The task brief names "Lyon et al. QEx 2016". QEx is **Ebke, Bommes, Campen, Kobbelt 2013** (S7.6);
   Lyon, Bommes, Kobbelt 2016 is HexEx (volumetric, `literature.md` B3.9).

---

## S0. Surveys and books (read first)

**S0.1 Bommes, Lévy, Pietroni, Puppo, Silva, Tarini, Zorin 2013, Quad-mesh generation and processing: a survey.** **Verdict: reference (read first for the graphics side).**
- **Citation.** CGF 32(6):51–76, 2013. DOI [10.1111/cgf.12014](https://doi.org/10.1111/cgf.12014). *Read:* metadata.
- **Use.** Taxonomy of field-guided, parametrization-based, Voronoi/patch and remeshing methods; the
  vocabulary (cross field, singularity, integer-grid map, base complex) used in S4–S7.

**S0.2 Campen 2017, Partitioning surfaces into quadrilateral patches: a survey.** **Verdict: reference.**
- **Citation.** CGF 36(8):567–588, 2017. DOI [10.1111/cgf.13153](https://doi.org/10.1111/cgf.13153). *Read:* metadata.
- **Use.** Layouts and block decompositions (S8), motorcycle graphs (S7.5).

**S0.3 Frey and George 2008, *Mesh Generation* (2nd ed.); Lo 2014, *Finite Element Mesh Generation*.** **Verdict: reference (engineering side).**
- **Citations.** P. J. Frey, P.-L. George, ISTE/Wiley 2008, DOI [10.1002/9780470611166](https://doi.org/10.1002/9780470611166).
  S. H. Lo, CRC Press 2014, DOI [10.1201/b17713](https://doi.org/10.1201/b17713). *Read:* metadata.
- **Use.** Parametric-surface meshing with the first fundamental form, metric Delaunay kernels, advancing
  front on surfaces, quad conversion. Lo is the author of S2.3–S2.4.

**S0.4 Vaxman, Campen, Diamanti, Panozzo, Bommes, Hildebrandt, Ben-Chen 2016, Directional field synthesis, design, and processing.** **Verdict: reference (cross fields, S4).**
- **Citation.** CGF 35(2):545–572, 2016. DOI [10.1111/cgf.12864](https://doi.org/10.1111/cgf.12864). *Read:* metadata.

---

## S1. Direct advancing front of quadrilaterals (paving family)

**S1.1 Blacker and Stephenson 1991, Paving.** **Verdict: reference (do not build).**
- **Citation.** T. D. Blacker, M. B. Stephenson. *Paving: a new approach to automated quadrilateral mesh
  generation.* IJNME 32(4):811–847, 1991. DOI [10.1002/nme.1620320410](https://doi.org/10.1002/nme.1620320410). *Read:* metadata (well known).
- **Method.** Rows of quads are laid from the boundary inward. Front nodes are classified by their interior
  angle (row end, side, corner, reversal); rows are adjusted (wedges, tucks) for size changes; small angles
  are seamed; colliding fronts are detected by intersection tests and connected; the last void (six nodes or
  fewer) is closed by templates; then cleanup and smoothing.
- **Solves.** Excellent boundary-parallel rows, few irregular nodes near the boundary: what analysts want
  around holes and edges.
- **Limits.** Front collision and intersection handling is the failure mode, and each case is a special
  case. Every operation decides on floating-point geometry with no background structure to fall back on.
- **Complexity.** High implementation complexity; intersection tests dominate run time.
- **Code.** Implemented in Sandia CUBIT (not open source). None readable.

**S1.2 Cass, Benzley, Meyers, Blacker 1996, Generalized 3-D paving.** **Verdict: reference.**
- **Citation.** *Generalized 3-D paving: an automated quadrilateral surface mesh generation algorithm.*
  IJNME 39(9):1475–1489, 1996. DOI [10.1002/(SICI)1097-0207(19960515)39:9<1475::AID-NME913>3.0.CO;2-W](https://doi.org/10.1002/(SICI)1097-0207(19960515)39:9%3C1475::AID-NME913%3E3.0.CO;2-W). *Read:* metadata.
- **Method.** Paving directly on 3D surfaces: front directions from surface normals and tangents, every new
  node projected onto the surface, collision tests in 3D.
- **Relevance.** The 3D alternative to meshing in (u,v). It avoids parametrization distortion but pays with
  projections and 3D intersection tests; in (u,v) the same tests become exact 2D predicates.

**S1.3 White and Kinney 1997, Redesign of the paving algorithm.** **Verdict: reference.**
- **Citation.** D. R. White, P. Kinney. *Redesign of the paving algorithm: robustness enhancements through
  element by element meshing.* Proc. 6th International Meshing Roundtable, pp. 323–335, 1997. No DOI; the
  citation and page range are as listed in Sandia's CUBIT reference list (not fetched as PDF). *Read:* metadata.
- **Method.** Elements added one at a time instead of whole rows, which simplifies the bookkeeping and the
  collision cases.
- **Relevance.** Even the paving authors' group moved away from row-at-a-time logic for robustness.

**S1.4 Staten, Owen, Blacker 2005, Unconstrained paving and plastering.** **Verdict: reference.**
- **Citations.** IMR 14 (2005) pp. 399–416, DOI [10.1007/3-540-29090-7_24](https://doi.org/10.1007/3-540-29090-7_24);
  journal version (plastering) IJNME 81(2):135–171, 2010, DOI [10.1002/nme.2679](https://doi.org/10.1002/nme.2679). *Read:* metadata.
- **Method.** The front bounds an unmeshed *geometric* void first; elements are formed later. It addresses
  collision by deferring connectivity.
- **Relevance.** Historical: it shows the collision problem was still open for the direct family in 2005–2010.

**S1.5 Ollivier-Gooch et al. 2025, Entire domain advancing-layer meshing. Part 1: two-dimensional and surface meshing.** **Verdict: watch.**
- **Citation.** SSRN preprint, 2025. DOI [10.2139/ssrn.5802502](https://doi.org/10.2139/ssrn.5802502) (Crossref lists
  first author Ollivier-Gooch). *Read:* metadata only; not peer reviewed.
- **Relevance.** A current advancing-layer approach covering whole surfaces, related to boundary-layer quads
  (a UBC thesis, "EDAMSurf", is in the same line). Re-check when published.

---

## S2. Indirect advancing front (Q-Morph and the Lo / Lee school)

**S2.1 Owen, Staten, Canann, Saigal 1999, Q-Morph.** **Verdict: reference (the ideas we keep: front states, layer order, side-edge choice; the implementation we don't build first).**
- **Citation.** *Q-Morph: an indirect approach to advancing front quad meshing.* IJNME 44(9):1317–1340, 1999.
  DOI [10.1002/(SICI)1097-0207(19990330)44:9<1317::AID-NME532>3.0.CO;2-N](https://doi.org/10.1002/(SICI)1097-0207(19990330)44:9%3C1317::AID-NME532%3E3.0.CO;2-N). *Read:* metadata (well known; IMR 1998 version: *Advancing front quadrilateral meshing using triangle transformations*).
- **Method.** Start from a triangulation. Front edges carry a state (how many of the two side edges exist).
  For each front edge, choose side edges (an existing front edge when the angle allows, otherwise the
  triangle edge closest to the bisector, made by swaps or splits), recover the top edge by edge swaps,
  delete the enclosed triangles to form a quad, smooth locally, update the front. Fronts are processed layer
  by layer; seams close small angles; transition splits handle size changes.
- **Solves.** Paving-quality boundary rows with collision handled by the background triangulation: opposing
  fronts meet in triangles that are always there.
- **Limits.** Many cases (state transitions, edge recovery failing, odd closure leaving a triangle). On
  surfaces it inherits the triangulation's quality and needs the recovery swaps to be valid on the surface.
- **Complexity.** O(n) local operations after the triangulation; high implementation complexity.
- **Code.** CUBIT (closed). Gmsh is not involved. None readable.

**S2.2 Owen and Saigal 2000, Surface mesh sizing control.** **Verdict: build on (size field on faces).**
- **Citation.** IJNME 47(1–3):497–511, 2000. DOI [10.1002/(SICI)1097-0207(20000110/30)47:1/3<497::AID-NME781>3.0.CO;2-H](https://doi.org/10.1002/(SICI)1097-0207(20000110/30)47:1/3%3C497::AID-NME781%3E3.0.CO;2-H). *Read:* metadata.
- **Method.** Automatic sizing on CAD surfaces from curvature and proximity, stored on a background mesh.
- **Use.** The curvature and proximity terms of FieldCraft's surface size field (P3.3).

**S2.3 Lo 1989; Lau, Lo, Lee 1997; Lo and Lau 1998: quadrilaterals over curved surfaces.** **Verdict: reference.**
- **Citations.**
  - S. H. Lo. *Generating quadrilateral elements on plane and over curved surfaces.* C&S 31(3):421–426, 1989.
    DOI [10.1016/0045-7949(89)90389-1](https://doi.org/10.1016/0045-7949(89)90389-1).
  - T. S. Lau, S. H. Lo, C. K. Lee. *Generation of quadrilateral mesh over analytical curved surfaces.* FEAD
    27(3):251–272, 1997. DOI [10.1016/S0168-874X(97)00015-2](https://doi.org/10.1016/S0168-874X(97)00015-2).
  - S. H. Lo, T. S. Lau. *Mesh generation over curved surfaces with explicit control on discretization error.*
    Eng. Comput. 15(3):357–373, 1998. DOI [10.1108/02644409810208516](https://doi.org/10.1108/02644409810208516).
  - *Read:* metadata for all three.
- **Method.** Lo's line: advancing-front triangulation then merging into quads, carried onto curved surfaces
  through their parametrization; the 1998 paper sizes elements by chordal (discretization) error.
- **Use.** Engineering precedent for exactly our problem (quads on analytic and parametric surfaces). The
  chordal-error sizing is P3.3's curvature term.

**S2.4 C. K. Lee: metric advancing front on surfaces; indirect anisotropic quads.** **Verdict: build on (metric formulation); reference (algorithms).**
- **Citations.**
  - C. K. Lee, S. H. Lo. *A new scheme for the generation of a graded quadrilateral mesh.* C&S 52(5):847–857,
    1994. DOI [10.1016/0045-7949(94)90070-1](https://doi.org/10.1016/0045-7949(94)90070-1) (already the basis of `quad::free`).
  - C. K. Lee, R. E. Hobbs. *Automatic adaptive finite element mesh generation over rational B-spline
    surfaces.* C&S 69(5):577–608, 1998. DOI [10.1016/S0045-7949(98)80096-5](https://doi.org/10.1016/S0045-7949(98)80096-5).
  - C. K. Lee. *Automatic adaptive mesh generation using metric advancing front approach.* Eng. Comput.
    16(2):230–263, 1999. DOI [10.1108/02644409910257494](https://doi.org/10.1108/02644409910257494).
  - C. K. Lee. *Automatic metric 3D surface mesh generation using subdivision surface geometrical model*,
    Parts 1–2. IJNME 56(11):1593–1614 and 1615–1646, 2003. DOIs [10.1002/nme.630](https://doi.org/10.1002/nme.630),
    [10.1002/nme.631](https://doi.org/10.1002/nme.631).
  - Y. K. Lee, C. K. Lee. *A new indirect anisotropic quadrilateral mesh generation scheme with enhanced local
    mesh smoothing procedures.* IJNME 58(2):277–300, 2003. DOI [10.1002/nme.771](https://doi.org/10.1002/nme.771).
  - *Read:* metadata for all.
- **Method.** Advancing front in the (u,v) domain where every length is measured in a Riemannian metric built
  from the surface's first fundamental form and the target size; then triangle merging into quads, with
  anisotropic (stretched) quads from an anisotropic metric.
- **Use.** The metric-in-(u,v) formulation of P3.2 is this one (and Tristano 1998, S3.1).

**S2.5 Zhu, Zienkiewicz, Hinton, Wu 1991; Johnston, Sullivan, Kwasnik 1991: early indirect methods.** **Verdict: reference.**
- **Citations.** Zhu et al., *A new approach to the development of automatic quadrilateral mesh generation*,
  IJNME 32(4):849–866, 1991, DOI [10.1002/nme.1620320411](https://doi.org/10.1002/nme.1620320411). Johnston et al.,
  *Automatic conversion of triangular finite element meshes to quadrilateral elements*, IJNME 31(1):67–84,
  1991, DOI [10.1002/nme.1620310105](https://doi.org/10.1002/nme.1620310105). *Read:* metadata.
- **Method.** Zhu: advancing-front triangles generated in pairs and merged as the front advances (a precursor
  of Q-Morph's "front of quads over triangles"). Johnston: post-hoc conversion of a triangulation by merging
  and splitting.

**S2.6 Pellenard, Orbay, Chen, Sohan, Kwok, Tristano 2014, QMCF: Q-Morph cross field-driven quad-dominant meshing.** **Verdict: reference (closest published match to "Q-Morph + cross field").**
- **Citation.** Procedia Engineering 82:338–350, 2014 (IMR 23). DOI [10.1016/j.proeng.2014.10.395](https://doi.org/10.1016/j.proeng.2014.10.395). Open access (CC BY-NC-ND). *Read:* abstract (via search snippets; the publisher page refused a scripted client).
- **Method.** Input: a triangulation of CAD surfaces and a size field. A cross field from principal curvature
  directions and boundary constraints steers Q-Morph: new front edges follow the cross field instead of
  growing orthogonally from the previous front, and fronts may start at interior points. Output is
  quad-dominant.
- **Relevance.** Industrial authors (one of them co-wrote the 1998 parametric-space paper S3.1). Evidence
  that the 2010s path for Q-Morph was "add a cross field", and that the result was quad-dominant, not
  all-quad, without a further step. No quantitative robustness data seen.

---

## S3. Meshing in parameter space with a metric (the surface substrate)

**S3.1 Tristano, Owen, Canann 1998, Advancing front surface mesh generation in parametric space using a Riemannian surface definition.** **Verdict: build on.**
- **Citation.** Proc. 7th International Meshing Roundtable, 1998. PDF (HTTP 200): <https://www.ljll.fr/~frey/papers/meshing/Tristano%20J.R.,%20Advancing%20front%20surface%20mesh%20generation%20in%20parametric%20space%20using%20a%20Riemannian%20surface%20definition.pdf>. *Read:* abstract.
- **Method.** Meshing happens in (u,v); lengths and angles are measured with the metric of the first
  fundamental form so elements that are distorted in (u,v) are well shaped on the surface.
- **Use.** P3.2: our metric is `M(u,v) = JᵀJ / h(S(u,v))²`, J = [S_u S_v].

**S3.2 Borouchaki, George, Hecht, Laug, Saltel 1997, Delaunay mesh generation governed by metric specifications.** **Verdict: build on (only if non-conformal parametrizations need an anisotropic Delaunay kernel).**
- **Citation.** Part I: FEAD 25(1–2):61–83, 1997, DOI [10.1016/S0168-874X(96)00057-1](https://doi.org/10.1016/S0168-874X(96)00057-1); Part II: FEAD 25:85–109, DOI [10.1016/S0168-874X(96)00065-0](https://doi.org/10.1016/S0168-874X(96)00065-0). *Read:* metadata.
- **Method.** The Delaunay criterion evaluated in a local metric (anisotropic Bowyer–Watson cavity).
- **Caveat for FieldCraft.** A metric in-circle test is not an exact predicate of the input coordinates; it
  may only decide *point placement* and *which flip to try*, never validity. Validity stays an exact `orient2d`
  sign in (u,v) (see P3.5).

**S3.3 Borouchaki, Laug, George 2000, Parametric surface meshing using a combined advancing-front generalized-Delaunay approach.** **Verdict: reference.**
- **Citation.** IJNME 49(1–2):233–259, 2000. DOI [10.1002/1097-0207(20000910/20)49:1/2<233::AID-NME931>3.0.CO;2-G](https://doi.org/10.1002/1097-0207(20000910/20)49:1/2%3C233::AID-NME931%3E3.0.CO;2-G). *Read:* metadata.
- **Method.** Frontal point placement with a generalized (metric) Delaunay kernel in (u,v): the "frontal-Delaunay"
  hybrid that S5 builds on. Companion: Borouchaki and Frey, *Adaptive triangular–quadrilateral mesh
  generation*, IJNME 41(5):915–934, 1998, DOI [10.1002/(SICI)1097-0207(19980315)41:5<915::AID-NME318>3.0.CO;2-Y](https://doi.org/10.1002/(SICI)1097-0207(19980315)41:5%3C915::AID-NME318%3E3.0.CO;2-Y)
  (metric triangulation then merging; 97% quads, min quality 0.23 on the test reported in S5.1).

**S3.4 Rebay 1993, Efficient unstructured mesh generation by means of Delaunay triangulation and Bowyer–Watson algorithm.** **Verdict: build on (the frontal-Delaunay loop).**
- **Citation.** JCP 106(1):125–138, 1993. DOI [10.1006/jcph.1993.1097](https://doi.org/10.1006/jcph.1993.1097). *Read:* metadata.
- **Method.** Triangles classified as resolved / active (front) / waiting; a new point is placed to make an
  ideal triangle on the front edge of the worst active triangle, inserted with Bowyer–Watson. A valid
  triangulation exists at every step, so fronts never "collide": they meet inside a triangulation.
- **Use.** The loop of S5.1 and of our P3.4. The existing `quad::cdt` already has Bowyer–Watson.

**S3.5 Cuillière 1998, An adaptive method for the automatic triangulation of 3D parametric surfaces.** **Verdict: reference.**
- **Citation.** CAD 30(2):139–149, 1998. DOI [10.1016/S0010-4485(97)00085-7](https://doi.org/10.1016/S0010-4485(97)00085-7). *Read:* metadata.

**S3.6 Remacle and Geuzaine 2022, Gmsh's approach to robust mesh generation of surfaces with irregular parametrizations.** **Verdict: build on (pole handling). Paper only (Gmsh is GPL).**
- **Citation.** In *Mesh Generation and Adaptation*, SEMA SIMAI Springer Series 30, pp. 95–112, 2022.
  DOI [10.1007/978-3-030-92540-6_5](https://doi.org/10.1007/978-3-030-92540-6_5). IMR 2019 version (HTTP 200):
  <https://www.internationalmeshingroundtable.com/assets/papers/2019/23-Remacle.pdf>. *Read:* abstract.
- **Problem.** Singular parametrizations from CAD (sphere poles, cone apexes, surfaces of revolution touching
  their axis): one boundary edge of the (u,v) domain maps to one 3D point.
- **Method.** Triangles at the singular point are treated as trapezoids with one collapsed edge.
- **Use.** Directly relevant: SolidCraft spheres and revolved faces have poles. For an all-quad mesh, the
  elements at a pole become degenerate quads; P3.1 gives our handling (chart switch) and this is the
  reference for the alternative.

**S3.7 Remacle et al. 2010; Marchandise et al. 2011, 2012; Beaufort, Geuzaine, Remacle 2020: reparametrization.** **Verdict: reference (fallback chart for faces whose native parametrization is unusable). Papers only.**
- **Citations.** Remacle, Geuzaine, Compère, Marchandise, *High-quality surface remeshing using harmonic
  maps*, IJNME 83(4):403–425, 2010, DOI [10.1002/nme.2824](https://doi.org/10.1002/nme.2824). Marchandise, Remacle, Geuzaine,
  *Optimal parametrizations for surface remeshing*, Eng. Comput. 30(3):383–402, 2014 (online 2012),
  DOI [10.1007/s00366-012-0309-3](https://doi.org/10.1007/s00366-012-0309-3). Beaufort, Geuzaine, Remacle, *Automatic surface
  mesh generation for discrete models – a complete and automatic pipeline based on reparameterization*, JCP
  417:109575, 2020, DOI [10.1016/j.jcp.2020.109575](https://doi.org/10.1016/j.jcp.2020.109575), arXiv <https://arxiv.org/abs/2001.02542>. *Read:* abstract (2020).
- **Method (2020).** Split a triangulation into patches, give each a one-to-one discrete parametrization
  (mean-value coordinates; proved one-to-one iff every parametric triangle has positive area), then mesh in
  that chart.
- **Use.** The fallback chart (P3.1) when a face has no usable native (u,v): the face's tessellation from
  `BoundarySurface` gets a discrete chart. The one-to-one check is an exact `orient2d` per triangle.
  Floater, *Mean value coordinates*, CAGD 20(1):19–27, 2003, DOI [10.1016/S0167-8396(03)00002-5](https://doi.org/10.1016/S0167-8396(03)00002-5).

**S3.8 Chae, Park, Choi 2006, Mesh generation on poorly parameterized free-form surfaces.** **Verdict: reference (risk evidence).**
- **Citation.** Proc. IMechE B 220(2):295–306, 2006. DOI [10.1243/095440506X77634](https://doi.org/10.1243/095440506X77634).
  Also Chae and Kwon, *Quadrilateral mesh generation on trimmed NURBS surfaces*, KSME Int. J. 15(5):592–601,
  2001 (KoreaScience page HTTP 200: <https://koreascience.or.kr/article/JAKO200111920737275.page>). *Read:* abstract.
- **Problem.** NURBS with bad parametrization (uneven knots, near-degenerate derivatives) make (u,v) meshers
  fail or produce ill-conditioned meshes; their fix is to regenerate the surface.

---

## S4. Cross fields (orientation for alignment)

**S4.1 Knöppel, Crane, Pinkall, Schröder 2013, Globally optimal direction fields.** **Verdict: build on (field computation on curved faces).**
- **Citation.** ACM TOG 32(4), 2013. DOI [10.1145/2461912.2462005](https://doi.org/10.1145/2461912.2462005). *Read:* metadata.
- **Method.** The smoothest n-direction field (n = 4 for crosses) on a triangulated surface from one sparse
  linear solve (or a small eigenproblem) of a connection Laplacian; boundary alignment as a constraint.
  Handles curvature correctly through the discrete connection.
- **Code.** geometry-central (`nmwsharp/geometry-central`) is **MIT: READ OK** (it implements this paper).
- **Why this one.** One linear solve, no mixed-integer step; the connection makes it correct on curved faces
  where a plain (u,v) angle field is not.

**S4.2 Remacle et al. 2013 (cross field part), Beaufort et al. 2017, Viertel and Osting 2019: Ginzburg–Landau / harmonic cross fields.** **Verdict: reference (the CAD-meshing variant of S4.1).**
- **Citations.** Beaufort, Lambrechts, Henrotte, Geuzaine, Remacle, *Computing cross fields: a PDE approach
  based on the Ginzburg–Landau theory*, Procedia Eng. 203:219–231, 2017, DOI [10.1016/j.proeng.2017.09.799](https://doi.org/10.1016/j.proeng.2017.09.799).
  Viertel, Osting, *An approach to quad meshing based on harmonic cross-valued maps and the Ginzburg–Landau
  theory*, SISC 41(1):A452–A479, 2019, DOI [10.1137/17M1142703](https://doi.org/10.1137/17M1142703). *Read:* metadata.
- **Method.** Represent the cross by (cos 4θ, sin 4θ); solve Laplace with boundary-aligned Dirichlet data
  (Remacle 2013, S5.1: two Laplace solves, valid as-is only for conformal charts), or relax the unit-norm
  constraint with a Ginzburg–Landau penalty and alternate diffusion and normalization (the MBO-style scheme
  that Reberol 2021, S5.4, uses per CAD face on edges with Crouzeix–Raviart interpolation).

**S4.3 Ray, Vallet, Li, Lévy 2008; Palacios and Zhang 2007: N-symmetry field design.** **Verdict: reference.**
- **Citations.** Ray et al., ACM TOG 27(2), 2008, DOI [10.1145/1356682.1356683](https://doi.org/10.1145/1356682.1356683). Palacios, Zhang,
  *Rotational symmetry field design on surfaces*, ACM TOG 26(3), 2007 (DOI not re-checked; see Unverified).

**S4.4 Panozzo, Puppo, Tarini, Sorkine-Hornung 2014, Frame fields; Jiang et al. 2015, metric customization; Desobry et al. 2021, non-orthogonal frame fields.** **Verdict: reference (anisotropic shells later).**
- **Citations.** ACM TOG 33(4), 2014, DOI [10.1145/2601097.2601179](https://doi.org/10.1145/2601097.2601179). Jiang, Fang, Huang, Bao, Tong,
  Desbrun, ACM TOG 34(4), 2015, DOI [10.1145/2766927](https://doi.org/10.1145/2766927). Desobry, Coudert-Osmont, Corman, Ray, Sokolov,
  CAD 139:103081, 2021, DOI [10.1016/j.cad.2021.103081](https://doi.org/10.1016/j.cad.2021.103081). *Read:* metadata.
- **Use.** Anisotropic and non-orthogonal frames (stretched quads along fillets and stiffener runs). Not needed
  for isotropic shells v1.

**S4.5 Dielen et al. 2021; Dong et al. 2025 (NeurCross): learned cross fields.** **Verdict: ignore for now.**
- **Citations.** CGF 40(5):181–191, 2021, DOI [10.1111/cgf.14366](https://doi.org/10.1111/cgf.14366); ACM TOG 44(4), 2025,
  DOI [10.1145/3731159](https://doi.org/10.1145/3731159). No determinism or robustness guarantee; trained on third-party data.

---

## S5. Cross-field-guided frontal point insertion (the modern "marching front")

**S5.1 Remacle, Henrotte, Carrier-Baudouin, Béchet, Marchandise, Geuzaine, Mouton 2013, A frontal Delaunay quad mesh generator using the L∞ norm ("Delquad").** **Verdict: BUILD ON (core of the point placement). Paper only: implementation is in Gmsh (GPL).**
- **Citation.** IJNME 94(5):494–512, 2013. DOI [10.1002/nme.4458](https://doi.org/10.1002/nme.4458). Preprint (HTTP 200):
  <http://gmsh.info/doc/preprints/gmsh_quad2_preprint.pdf>. *Read:* full (preprint; numbers below are the
  preprint's and may differ slightly in the journal version).
- **Method.**
  - §2: meshing in (u,v) with the metric tensor M = [S_u·S_u, S_u·S_v; S_v·S_u, S_v·S_v]; for a **conformal**
    parametrization the (u,v) size is δ'(u) = δ(x(u)) |det M|^(−1/4). Non-conformal faces are reparametrized
    (§2 says so explicitly).
  - §3: cross field by two Laplace solves for cos 4θ, sin 4θ with boundary-aligned Dirichlet data.
  - §4–§5: frontal-Delaunay (Rebay, S3.4) in which sizes and the "ideal" new point are measured in the
    **L∞ norm in the local cross-field frame**. A right isosceles triangle is equilateral in L∞, so the
    triangulation is made of nearly right triangles aligned with the field, with √3/2 ≈ 0.866 times the
    vertices of an equilateral one (the measured ratio was 0.87). Fronts are processed layer by layer.
    Connection uses the ordinary L2 Delaunay kernel (§5: the L∞ and L2 Delaunay triangulations differed by
    5 flips on their example).
  - §7: recombination with Blossom-Quad (S6.2).
- **Results (preprint).** Car hood (18 Bézier faces): Delquad + Blossom gives mean quad quality 0.90, worst
  0.37, 81% of vertices of valence 4, versus 0.77 / 0.37 / 71% for an ordinary frontal triangulation
  recombined the same way (Fig. 17). Mechanical part, 114 faces, 66,998 quads in 40 s including cross fields
  and reparametrization: mean 0.95, worst 0.1, 91% valence-4 (§8.1). Quality measure (§6.1):
  η = max(1 − (2/π)·max_k |π/2 − α_k|, 0).
- **Robustness.** Inherits Delaunay's "a valid mesh exists at every step": no front collision cases.
- **Limits.** Conformal-parametrization assumption; recombination may leave triangles unless a perfect
  matching exists; irregular vertices where fronts meet (§7).
- **Complexity.** Same as frontal-Delaunay triangulation, O(n log n); "tens of thousands of elements a minute"
  (§1) in 2010-era hardware.
- **Why it matters here.** It is a marching front, but one whose collisions cannot fail, and it is a small
  change to machinery FieldCraft already has (Bowyer–Watson, constrained recovery, recombination).

**S5.2 Georgiadis, Beaufort, Lambrechts, Remacle 2017, High quality mesh generation using cross and asterisk fields.** **Verdict: reference.**
- **Citation.** arXiv <https://arxiv.org/abs/1706.02236>, 2017. *Read:* abstract.
- **Method.** Points generated by an advancing front *along the cross field* using only the background mesh,
  multi-threaded; right-angled triangulations recombined with Blossom-Quad, with a right-triangle quality
  criterion relative to the local cross.

**S5.3 Pellenard et al. 2014, QMCF.** See S2.6 (the Q-Morph route to the same goal).

**S5.4 Reberol, Georgiadis, Remacle 2021, Quasi-structured quadrilateral meshing in Gmsh: a robust pipeline for complex CAD models.** **Verdict: BUILD ON (the pipeline shape and the cleanup). Paper only (Gmsh is GPL).**
- **Citation.** arXiv <https://arxiv.org/abs/2103.04652>, 2021 (no journal version found in Crossref/DBLP). *Read:* full (HTML version).
- **Pipeline (§1.1).** (1) scaled cross field per CAD face on an initial triangulation (§3: MBO-style heat
  diffusion + normalization, multi-level, boundary-aligned Dirichlet; conformal scaling H from
  min ∫|∇H − ∇⊥θ|², size h = e^H); (2) global size map from prescribed sizes, small CAD features and the
  cross-field scaling (§4); (3) mesh CAD curves under topological constraints (§4.3); (4) mesh simple faces
  matching predefined patterns (§6.1.2); (5) remaining faces: frontal point insertion along the scaled cross
  field with a Delaunay kernel, greedy recombination (not Blossom) by quad quality weighted by alignment,
  merges accepted while SICN > 0.1, then midpoint subdivision (quad → 4, triangle → 3) with snapping to the
  CAD surface and revert on negative quality (§5); smoothing in (u,v) (§5.3); (6) topology changes to remove
  irregular vertices that do not match cross-field singularities: local *disk quadrangulations* (§6.3,
  valence-driven, interior valence 3–5) and cavity remeshing with predefined patterns (§6.2). Every operation
  is reverted when it invalidates or degrades elements.
- **Results (§7, Table 1).** MAMBO 114 models: min SICN 0.53, mean 0.98, irregular vertices 1.24% → 0.27%,
  9.6 s + 4.6 s; all 114 valid. ABC 1000 random "not too simple" models: min SICN 0.44, mean 0.96, irregular
  2.2% → 0.74%; 988/1000 valid (2 tangled, 10 Delaunay-insertion or resource failures); median whole pipeline
  18 s single-threaded.
- **Limits (§8, and in text).** Irregular vertices left by curve quantization cannot be removed within one face;
  slow on faces over 100k vertices; acute corners (< 45°) need artificial singularities; size-map vs
  cross-field compatibility "still open".
- **Why it matters here.** Strongest published evidence on CAD B-reps for any automatic all-quad method, and
  it uses **the same tri → recombine → split guarantee FieldCraft already ships**, with better point placement
  in front of it and cleanup behind it. It is the template for P.
- **Data caveat.** Both datasets are excluded for our own tests (see `literature.md` Part C); we cite the numbers only.

**S5.5 Tchon and Camarero 2006; Lévy and Liu 2010; Ekelschot et al. 2018: L∞/Lp point optimization for quad-dominant meshes.** **Verdict: reference.**
- **Citations.** Tchon, Camarero, *Quad-dominant mesh adaptation using specialized simplicial optimization*,
  IMR 15, pp. 21–38, DOI [10.1007/978-3-540-34958-7_2](https://doi.org/10.1007/978-3-540-34958-7_2). Lévy, Liu, *Lp centroidal Voronoi tessellation and
  its applications*, ACM TOG 29(4), 2010, DOI [10.1145/1778765.1778856](https://doi.org/10.1145/1778765.1778856) (geogram, BSD-3 core:
  READ OK). Ekelschot, Ceze, Garai, Murman, *Robust metric aligned quad-dominant meshing using Lp centroidal
  Voronoi tessellation*, AIAA SciTech 2018, DOI [10.2514/6.2018-1501](https://doi.org/10.2514/6.2018-1501). *Read:* metadata.
- **Relevance.** Global smoothing alternatives to frontal placement. S5.1 reports Tchon 84% quads and 101 s for
  972 quads, Lévy 322 s for ~1000 elements (preprint §1): far slower than frontal insertion.

---

## S6. Triangle-to-quad conversion

**S6.1 Lee and Lo 1994 + Catmull–Clark split (shipped in `quad::free`).** See `literature.md` B7.2. **Verdict: BUILD ON (keep as the all-quad guarantee).**
- The split halves the size: points are placed at 2h and the final mesh is at h. Reberol 2021 (S5.4) uses the
  same split ("midpoint subdivision") after a frontal, aligned point placement and reports the cleanup removes
  most of the irregular vertices it introduces.

**S6.2 Remacle et al. 2012, Blossom-Quad.** **Verdict: reference (optional upgrade of the matching). Paper only.**
- **Citation.** IJNME 89(9):1102–1119, 2012. DOI [10.1002/nme.3279](https://doi.org/10.1002/nme.3279). *Read:* metadata.
- **Note.** Reberol 2021 replaced Blossom with a greedy, alignment-weighted matching followed by the split, which
  removes the "perfect matching may not exist" failure and the cubic-time matching. That supports keeping
  FieldCraft's greedy matching.

**S6.3 Ramaswami, Ramos, Toussaint 1998, Converting triangulations to quadrangulations.** **Verdict: reference (theory).**
- **Citation.** CGTA 9(4):257–276, 1998. DOI [10.1016/S0925-7721(97)00019-9](https://doi.org/10.1016/S0925-7721(97)00019-9).

---

## S7. Global parametrization and field-aligned remeshing (graphics family)

Common property: they remesh a *triangle mesh* into quads aligned with a field. None natively takes prescribed
boundary nodes shared with neighbouring faces or parts, which is what conformity across B-rep edges needs.

**S7.1 Kälberer, Nieser, Polthier 2007, QuadCover.** **Verdict: reference.**
- **Citation.** CGF 26(3):375–384, 2007. DOI [10.1111/j.1467-8659.2007.01060.x](https://doi.org/10.1111/j.1467-8659.2007.01060.x).
- **Method.** Branched-covering parametrization from a cross field (curvature-aligned); integer grid lines give quads.
- **Code.** No official open-source implementation found; Polthier group's JavaView is not open source.

**S7.2 Ray, Li, Lévy, Sheffer, Alliez 2006, Periodic global parameterization.** **Verdict: reference.**
- **Citation.** ACM TOG 25(4):1460–1485, 2006. DOI [10.1145/1183287.1183297](https://doi.org/10.1145/1183287.1183297).

**S7.3 Bommes, Zimmer, Kobbelt 2009, Mixed-integer quadrangulation (MIQ).** **Verdict: reference.**
- **Citation.** ACM TOG 28(3), 2009. DOI [10.1145/1531326.1531383](https://doi.org/10.1145/1531326.1531383).
- **Method.** Cross field with integer period jumps, then a seamless parametrization by greedy mixed-integer
  rounding; feature curves become integer iso-lines.
- **Limits.** Greedy rounding can produce degenerate or folded charts; needs QEx (S7.6) to extract.
- **Code.** libigl `copyleft/comiso` (CoMISo, **GPL**: FORBIDDEN). libigl core is MPL-2.0 (READ OK) but GitHub
  reports GPL-3.0 for the repo because both licence files exist.

**S7.4 Bommes, Campen, Ebke, Alliez, Kobbelt 2013, Integer-grid maps for reliable quad meshing; Myles, Pietroni, Zorin 2014, Robust field-aligned global parametrization.** **Verdict: reference.**
- **Citations.** ACM TOG 32(4), 2013, DOI [10.1145/2461912.2462014](https://doi.org/10.1145/2461912.2462014). ACM TOG 33(4), 2014,
  DOI [10.1145/2601097.2601154](https://doi.org/10.1145/2601097.2601154).

**S7.5 Campen, Bommes, Kobbelt 2015, Quantized global parametrization; Campen and Zorin 2017; Lyon, Campen, Kobbelt 2021; Heistermann, Warnett, Bommes 2023; Coudert-Osmont et al. 2023.** **Verdict: reference (quantization is the robust part of this family now).**
- **Citations.**
  - Campen, Bommes, Kobbelt, ACM TOG 34(6), 2015, DOI [10.1145/2816795.2818140](https://doi.org/10.1145/2816795.2818140) (motorcycle-graph T-mesh, valid IGM by construction).
  - Campen, Zorin, *Similarity maps and field-guided T-splines: a perfect couple*, ACM TOG 36(4), 2017, DOI [10.1145/3072959.3073647](https://doi.org/10.1145/3072959.3073647).
  - Campen, Shen, Zhou, Zorin, *Seamless parametrization with arbitrary cones for arbitrary genus*, ACM TOG 39(1), 2019, DOI [10.1145/3360511](https://doi.org/10.1145/3360511).
  - Lyon, Campen, Kobbelt, *Quad layouts via constrained T-mesh quantization*, CGF 40(2):305–314, 2021, DOI [10.1111/cgf.142634](https://doi.org/10.1111/cgf.142634); *Simpler quad layouts using relaxed singularities*, CGF 40(5):169–180, 2021, DOI [10.1111/cgf.14365](https://doi.org/10.1111/cgf.14365).
  - Heistermann, Warnett, Bommes, *Min-deviation-flow in bi-directed graphs for T-mesh quantization*, ACM TOG 42(4), 2023, DOI [10.1145/3592437](https://doi.org/10.1145/3592437).
  - Coudert-Osmont, Desobry, Heistermann, Bommes, Ray, Sokolov, *Quad mesh quantization without a T-mesh*, CGF 43(1), 2024 (issue dated 2023 online), DOI [10.1111/cgf.14928](https://doi.org/10.1111/cgf.14928). *Read:* abstract (operates on a decimated surface instead of a T-mesh; "easier to implement and to adapt to constraints such as free boundaries, complex feature curves").
- **Code.** `cgg-bern/libsatsuma` (BiMDF solvers) **MIT: READ OK**; `cgg-bern/quadwild-bimdf` **GPL-3.0: FORBIDDEN**.

**S7.6 Ebke, Bommes, Campen, Kobbelt 2013, QEx: robust quad mesh extraction.** **Verdict: reference.**
- **Citation.** ACM TOG 32(6), 2013. DOI [10.1145/2508363.2508372](https://doi.org/10.1145/2508363.2508372).
- **Code.** `hcebke/libQEx` **GPL-3.0: FORBIDDEN**.

**S7.7 Lyon, Campen, Bommes, Kobbelt 2019, Parametrization quantization with free boundaries for trimmed quad meshing.** **Verdict: reference (relevant to trimmed faces, wrong output type).**
- **Citation.** ACM TOG 38(4), 2019. DOI [10.1145/3306346.3323019](https://doi.org/10.1145/3306346.3323019).
- **Relevance.** Quad meshes *trimmed* by the boundary (quads cut by trimming curves, not aligned to it). For
  FE we need boundary-conforming elements with shared boundary nodes, so this is not a direct fit.

**S7.8 Jakob, Tarini, Panozzo, Sorkine-Hornung 2015, Instant field-aligned meshes.** **Verdict: reference.**
- **Citation.** ACM TOG 34(6), 2015. DOI [10.1145/2816795.2818078](https://doi.org/10.1145/2816795.2818078).
- **Method.** Local smoothing of an orientation field and a position field (both with 4-RoSy / lattice
  symmetry), then extraction. Very fast, quad-dominant, may leave non-quad faces and defects.
- **Code.** `wjakob/instant-meshes` **BSD-3+: READ OK**.

**S7.9 Huang, Zhou, Nießner, Shewchuk, Guibas 2018, QuadriFlow.** **Verdict: reference.**
- **Citation.** CGF 37(5):147–160, 2018. DOI [10.1111/cgf.13498](https://doi.org/10.1111/cgf.13498).
- **Method.** Instant-Meshes-style fields with singularity reduction by min-cost flow plus a SAT step for
  orientation/position consistency.
- **Code.** `hjwdzh/QuadriFlow` **BSD-3+: READ OK**; bundled `3rd/MapleCOMSPS_LRB` (SAT solver) and `lemon`
  have their own licences, not checked: do not open those directories.

**S7.10 Pietroni, Nuvoli, Alderighi, Cignoni, Tarini 2021, Reliable feature-line driven quad-remeshing (QuadWild).** **Verdict: reference.**
- **Citation.** ACM TOG 40(4), 2021. DOI [10.1145/3450626.3459941](https://doi.org/10.1145/3450626.3459941). PDF (author copy):
  <https://vcg-legacy.isti.cnr.it/Publications/2021/PNACT21/ReliableQuad.pdf>. *Read:* abstract.
- **Method.** Field-traced patch layout respecting feature lines, patches quantized by an ILP, each patch
  quadrangulated. Reported failure rate < 0.5% over Thingi10K.
- **Code.** `nicopietroni/quadwild` **GPL-3.0: FORBIDDEN** (depends on vcglib, also GPL-3.0).
- **Relevance.** The most robust remesher in this family, but a remesher of triangle meshes; boundary nodes
  are not prescribed.

**S7.11 Alliez et al. 2003; Marinov and Kobbelt 2006; Zhang et al. 2010: curvature-line and wave-based quad-dominant remeshing.** **Verdict: reference.**
- **Citations.** *Anisotropic polygonal remeshing*, ACM TOG 22(3):485–493, 2003, DOI [10.1145/882262.882296](https://doi.org/10.1145/882262.882296).
  *A robust two-step procedure for quad-dominant remeshing*, CGF 25(3):537–546, 2006, DOI [10.1111/j.1467-8659.2006.00973.x](https://doi.org/10.1111/j.1467-8659.2006.00973.x).
  *A wave-based anisotropic quadrangulation method*, ACM TOG 29(4), 2010, DOI [10.1145/1778765.1778855](https://doi.org/10.1145/1778765.1778855).

---

## S8. Layouts, block decomposition and mapped/submapped faces

**S8.1 Tam and Armstrong 1991, medial axis subdivision; Fogg, Armstrong, Robinson 2015, multiblock decompositions of surfaces.** **Verdict: reference (structured shells later).**
- **Citations.** Adv. Eng. Softw. 13(5–6):313–324, 1991, DOI [10.1016/0961-3552(91)90035-3](https://doi.org/10.1016/0961-3552(91)90035-3). IJNME 101(13):965–991, 2015,
  DOI [10.1002/nme.4825](https://doi.org/10.1002/nme.4825).
- **Relevance.** The Belfast group's route for shell structures: decompose into four-sided blocks, map each.

**S8.2 Whiteley, White, Benzley, Blacker 1996, submapping; Ruiz-Gironés and Sarrate 2010.** **Verdict: BUILD ON (mapped faces first).**
- **Citations.** *Two and three-quarter dimensional meshing facilitators*, Eng. Comput. 12:144–154, 1996,
  DOI [10.1007/BF01198730](https://doi.org/10.1007/BF01198730). *Generation of structured meshes in multiply connected surfaces using submapping*, Adv. Eng. Softw.
  41(2):379–387, 2010, DOI [10.1016/j.advengsoft.2009.06.009](https://doi.org/10.1016/j.advengsoft.2009.06.009). *Read:* metadata.
- **Use.** Many SolidCraft faces are four-sided in (u,v) (cylinder walls, fillets, extrusion sides). TFI
  (`quad::tfi`) in (u,v) with metric-aware boundary spacing gives perfect quads there; Reberol 2021 step 4 does
  the same with "patterns". Submapping extends this to faces with corners at multiples of 90°.

**S8.3 Kowalski, Ledoux, Frey 2013; Jezdimirović et al. 2021: PDE- and singularity-driven partitions.** **Verdict: reference.**
- **Citations.** IMR 21, pp. 137–154, DOI [10.1007/978-3-642-33573-0_9](https://doi.org/10.1007/978-3-642-33573-0_9). *Quad layouts with high valence
  singularities for flexible quad meshing*, arXiv <https://arxiv.org/abs/2103.02939>, 2021. *Read:* abstract (2021).

**S8.4 Bunin 2008, A continuum theory for unstructured mesh generation in two dimensions.** **Verdict: reference (theory of where singularities must go).**
- **Citation.** CAGD 25(1):14–40, 2008. DOI [10.1016/j.cagd.2007.05.002](https://doi.org/10.1016/j.cagd.2007.05.002).

---

## S9. Quality improvement: topological cleanup, smoothing, untangling

**S9.1 Kinney 1997, CleanUp: improving quadrilateral finite element meshes.** **Verdict: BUILD ON (operation catalogue).**
- **Citation.** P. Kinney, Proc. 6th International Meshing Roundtable, pp. 437–447, 1997. PDF (HTTP 200):
  <https://people.eecs.berkeley.edu/~jrs/meshpapers/Kinney.pdf>. *Read:* abstract.
- **Method.** Heuristic connectivity cleanup (node valence), boundary and flange patterns, shape and size
  cleanup; used behind paving in CUBIT.

**S9.2 Canann, Muthukrishnan, Phillips 1998, Topological improvement procedures for quadrilateral finite element meshes.** **Verdict: BUILD ON.**
- **Citation.** Eng. Comput. 14(2):168–177, 1998. DOI [10.1007/BF01213591](https://doi.org/10.1007/BF01213591). *Read:* metadata.
- **Method.** Local operations (doublet removal, diagonal collapse/flip, edge swap, boundary-specific
  operations) to drive valences toward 4 (interior) and to the ideal boundary valence.

**S9.3 Reberol 2021 disk quadrangulations (S5.4 §6.3); Bommes, Lempfer, Kobbelt 2011, global structure optimization; Verma and Suresh 2015/2017.** **Verdict: BUILD ON (Reberol's local operator); reference (others).**
- **Citations.** Bommes et al., CGF 30(2):375–384, 2011, DOI [10.1111/j.1467-8659.2011.01868.x](https://doi.org/10.1111/j.1467-8659.2011.01868.x). Verma,
  Suresh, *A robust combinatorial approach to reduce singularities in quadrilateral meshes*, Procedia Eng.
  124:252–264, 2015, DOI [10.1016/j.proeng.2015.10.137](https://doi.org/10.1016/j.proeng.2015.10.137); CAD 85:99–110, 2017,
  DOI [10.1016/j.cad.2016.07.008](https://doi.org/10.1016/j.cad.2016.07.008). *Read:* metadata (Bommes, Verma).
- **Why Reberol's operator.** It is a single generic operation (replace a cavity by the best enumerated disk
  quadrangulation by an irregularity score, revert if quality drops), not a catalogue of special cases, and
  it has the published CAD-scale evidence (S5.4).

**S9.4 Daniels, Silva, Shepherd, Cohen 2008; Tarini et al. 2010: quad mesh simplification; Shepherd et al. 2010: adaptive coarsening.** **Verdict: reference.**
- **Citations.** ACM TOG 27(5), 2008, DOI [10.1145/1409060.1409101](https://doi.org/10.1145/1409060.1409101). CGF 29(2):407–418, 2010,
  DOI [10.1111/j.1467-8659.2009.01610.x](https://doi.org/10.1111/j.1467-8659.2009.01610.x). FEAD 46:17–32, 2010, DOI [10.1016/j.finel.2009.06.024](https://doi.org/10.1016/j.finel.2009.06.024).

**S9.5 Zhou and Shimada 2000, An angle-based approach to two-dimensional mesh smoothing.** **Verdict: BUILD ON (cheap smoother for quads).**
- **Citation.** Proc. 9th International Meshing Roundtable, pp. 373–384, 2000. PDF (HTTP 200):
  <https://www.andrew.cmu.edu/user/shimada/papers/00-imr-zhou.pdf>. *Read:* abstract.
- **Method.** Move each node to equalize adjacent angles; fewer inversions than Laplacian, much cheaper than
  optimization. On a surface: run in the tangent plane, map back through (u,v).
- Related: Xu, Newman, *An angle-based optimization approach for 2D finite element mesh smoothing*, FEAD
  42(13):1150–1164, 2006, DOI [10.1016/j.finel.2006.01.016](https://doi.org/10.1016/j.finel.2006.01.016).

**S9.6 Escobar, Montero, Montenegro, Rodríguez 2005/2006, smoothing surface triangulations on a local parametric space; Ruiz-Gironés et al. 2015, simultaneous untangling and smoothing of quads and hexes.** **Verdict: BUILD ON (worst-element optimization and untangling on surfaces).**
- **Citations.** IJNME 66:740–760, 2006 (online 2005), DOI [10.1002/nme.1584](https://doi.org/10.1002/nme.1584). Adv. Eng. Softw. 80:12–24, 2015,
  DOI [10.1016/j.advengsoft.2014.09.021](https://doi.org/10.1016/j.advengsoft.2014.09.021). *Read:* metadata.
- **Use.** Extends the planar Freitag–Plassmann untangling already in `quad::smooth` to nodes that move in
  (u,v) while quality is measured on the surface. Garanzha et al. 2021 (`literature.md` B6.4,
  DOI [10.1145/3450626.3459847](https://doi.org/10.1145/3450626.3459847)) is the global alternative.

---

## S10. Element quality for shells

**S10.1 Knupp 2001, Algebraic mesh quality metrics.** **Verdict: BUILD ON** (scaled Jacobian, shape; already used).
- SISC 23(1):193–218, 2001. DOI [10.1137/S1064827500371499](https://doi.org/10.1137/S1064827500371499).

**S10.2 Robinson 1987, Some new distortion measures for quadrilaterals; Yuan, Huang, Pian 1994.** **Verdict: BUILD ON (warp and taper for non-planar quads).**
- **Citations.** FEAD 3(3):183–197, 1987, DOI [10.1016/0168-874X(87)90023-0](https://doi.org/10.1016/0168-874X(87)90023-0). Yuan et al., *Inverse
  mapping and distortion measures for quadrilaterals with curved boundaries*, IJNME 37(5):861–875, 1994,
  DOI [10.1002/nme.1620370510](https://doi.org/10.1002/nme.1620370510).
- **Use.** A quad on a curved face is not planar. MITC4 accepts warped quads but the error grows with warp, so
  warp angle joins the report.

**S10.3 Dvorkin and Bathe 1984 (MITC4); Bathe, Iosilevich, Chapelle 2000, An evaluation of the MITC shell elements; Chapelle and Bathe 2011.** **Verdict: reference (what the mesh must serve).**
- **Citations.** Eng. Comput. 1(1):77–88, 1984, DOI [10.1108/eb023562](https://doi.org/10.1108/eb023562). C&S 75(1):1–30, 2000,
  DOI [10.1016/S0045-7949(99)00214-X](https://doi.org/10.1016/S0045-7949(99)00214-X). *The Finite Element Analysis of Shells – Fundamentals*, 2nd ed.,
  Springer 2011, DOI [10.1007/978-3-642-16408-8](https://doi.org/10.1007/978-3-642-16408-8).
- **Use.** Bathe et al. 2000 evaluate MITC elements on distorted meshes; that is the model for our "free mesh vs
  structured mesh" convergence comparison in P5.

---

## S11. Through-thickness: swept quads for continuum shells

**S11.1 Staten, Canann, Owen 1999, BMSweep; Roca and Sarrate 2010, least-squares projection for sweeping.** **Verdict: BUILD ON (M5/M8 sweep).**
- **Citations.** Eng. Comput. 15(3):212–218, 1999, DOI [10.1007/s003660050016](https://doi.org/10.1007/s003660050016). Eng. Comput. 26(4):391–406, 2010,
  DOI [10.1007/s00366-009-0172-z](https://doi.org/10.1007/s00366-009-0172-z). *Read:* metadata.
- **Use.** A thin-walled solid: mesh the source skin with the shell quad mesher, sweep to the target skin
  (one-to-one sweep), side faces mapped by TFI with n layers. The source and target must share the source's
  topology, so the target is *not* meshed independently.

**S11.2 Hauptmann and Schweizerhof 1998; Vu-Quoc and Tan 2003: solid-shell elements.** **Verdict: reference (the element on top, M8).**
- **Citations.** IJNME 42(1):49–69, 1998, DOI [10.1002/(SICI)1097-0207(19980515)42:1<49::AID-NME349>3.0.CO;2-2](https://doi.org/10.1002/(SICI)1097-0207(19980515)42:1%3C49::AID-NME349%3E3.0.CO;2-2).
  CMAME 192(9–10):975–1016, 2003, DOI [10.1016/S0045-7825(02)00435-8](https://doi.org/10.1016/S0045-7825(02)00435-8).
- **Mesh requirements they imply.** One or more hex8 layers through the thickness; a consistent stacking
  direction (bottom face nodes 1–4, top 5–8, normal pointing the same way across the part); the in-plane
  quad quality of the swept mesh is the shell mesher's.

**S11.3 Robinson, Armstrong, Fairey 2011, Automated mixed dimensional modelling from 2D and 3D CAD models.** **Verdict: reference (midsurface, M8).**
- **Citation.** FEAD 47(2):151–165, 2011. DOI [10.1016/j.finel.2010.08.010](https://doi.org/10.1016/j.finel.2010.08.010).

---

## S12. Shell benchmarks (verification of mesh + element)

| Benchmark | Source | Use |
|---|---|---|
| Scordelis–Lo roof, pinched cylinder with rigid diaphragms, pinched hemisphere with 18° hole, twisted beam, patch tests on distorted elements | MacNeal, Harder, *A proposed standard set of problems to test finite element accuracy*, FEAD 1(1):3–20, 1985, DOI [10.1016/0168-874X(85)90003-4](https://doi.org/10.1016/0168-874X(85)90003-4); Belytschko, Stolarski, Liu, Carpenter, Ong, *Stress projection for membrane and shear locking in shell finite elements*, CMAME 51(1–3):221–258, 1985, DOI [10.1016/0045-7825(85)90035-0](https://doi.org/10.1016/0045-7825(85)90035-0) (the "shell obstacle course") | Linear statics; mesh convergence on our free meshes vs the published reference values |
| Geometrically nonlinear shell set | Sze, Liu, Lo, FEAD 40(11):1551–1569, 2004, DOI [10.1016/j.finel.2003.11.001](https://doi.org/10.1016/j.finel.2003.11.001) (Crossref also holds a duplicate DOI `10.1016/S0168-874X(03)00218-X` for the same paper) | Later (nonlinear steps) |

Reference values as commonly quoted (confirm from the source PDFs before freezing a fixture): Scordelis–Lo
midside free-edge vertical displacement 0.3024 (obstacle course; MacNeal–Harder quote 0.3086); pinched
cylinder displacement under the load 1.8248×10⁻⁵ (E = 3×10⁶, P = 1); pinched hemisphere 0.094.

---

## S13. Guaranteed-quality theory and other all-quad methods

**S13.1 Liang, Ebeida, Zhang 2010; Bishop 2016; Rushdi et al. 2017.** **Verdict: reference.**
- **Citations.** Liang et al., *Guaranteed-quality all-quadrilateral mesh generation with feature
  preservation*, CMAME 199(29–32):2072–2083, 2010, DOI [10.1016/j.cma.2010.03.007](https://doi.org/10.1016/j.cma.2010.03.007) (quadtree, angle bounds;
  IMR 18 version DOI [10.1007/978-3-642-04319-2_4](https://doi.org/10.1007/978-3-642-04319-2_4)). Bishop, *Quadrilateral meshes for PSLGs*, DCG 56(1):1–42,
  2016, DOI [10.1007/s00454-016-9771-9](https://doi.org/10.1007/s00454-016-9771-9). Rushdi, Mitchell, Mahmoud, Bajaj, Ebeida, *All-quad meshing without
  cleanup*, CAD 85:83–98, 2017, DOI [10.1016/j.cad.2016.07.009](https://doi.org/10.1016/j.cad.2016.07.009). *Read:* metadata.
- **Relevance.** Provable angle bounds, but quadtree orientation artefacts at the boundary (S5.1 Fig. 2 shows
  the 45°-rotated boundary quads) and no boundary rows: wrong trade for shells.

**S13.2 Sarrate and Huerta 2000, Efficient unstructured quadrilateral mesh generation.** **Verdict: reference.**
- IJNME 49(10):1327–1350, 2000. DOI [10.1002/1097-0207(20001210)49:10<1327::AID-NME996>3.0.CO;2-L](https://doi.org/10.1002/1097-0207(20001210)49:10%3C1327::AID-NME996%3E3.0.CO;2-L). Recursive domain splitting.

**S13.3 Learning-based quad meshing (Pan et al. 2023; Tong et al. 2023).** **Verdict: ignore.**
- Pan, Huang, Cheng, Zeng, Neural Networks 157:288–304, 2023, DOI [10.1016/j.neunet.2022.10.022](https://doi.org/10.1016/j.neunet.2022.10.022). Tong, Qian,
  Halilaj, Zhang, arXiv <https://arxiv.org/abs/2305.00540> (planar only; trained to predict a commercial
  mesher's output, which also rules it out under our rules).

**S13.4 Owen and Saigal 2000, H-Morph.** **Verdict: reference** (the hex analogue of Q-Morph). IJNME 49(1–2):289–312,
2000, DOI [10.1002/1097-0207(20000910/20)49:1/2<289::AID-NME934>3.0.CO;2-L](https://doi.org/10.1002/1097-0207(20000910/20)49:1/2%3C289::AID-NME934%3E3.0.CO;2-L).

---

## Where the field is today versus 2000s paving/Q-Morph

- **2000s:** direct paving and Q-Morph gave the best boundary rows; robustness came from heuristics plus
  CleanUp (Kinney) and topological operations (Canann), and fronts colliding in odd configurations was the
  standing failure mode. Surface meshing was done in (u,v) with a first-fundamental-form metric (Tristano;
  Lee; Borouchaki–Laug–George).
- **2010s:** orientation moved from "whatever the front does" to an explicit **cross field** computed first
  (Remacle 2013 Laplace; Knöppel 2013; Ginzburg–Landau 2017–2019). Fronts became **frontal point insertion
  over a Delaunay kernel** (Delquad), so collisions cannot fail, and recombination became the all-quad step
  (Blossom-Quad; or greedy matching + split). In graphics, global parametrization (MIQ → IGM → QGP) matured,
  but with mixed-integer solves and GPL code, and as remeshers of triangle meshes.
- **2020s:** the robust CAD pipeline is Reberol 2021: aligned frontal points → recombination → midpoint split
  (all-quad guarantee) → singularity-aware topological cleanup with quality-monotone reverts. Published: 100%
  valid on 114 MAMBO models, 98.8% on 1000 ABC models, 0.27–0.74% irregular vertices, min SICN 0.44–0.53.
  Quantization research (Lyon 2021, Heistermann 2023, Coudert-Osmont 2023) targets *block-structured* layouts;
  QuadWild (< 0.5% failures on Thingi10K) is the most robust triangle-mesh remesher. Q-Morph-with-a-cross-field
  exists (QMCF 2014) but I found no evidence it outperforms the frontal-Delaunay route, and nothing new on
  direct paving beyond preprints.

---

## Part P: Recommended plan for FieldCraft shell meshing (opinionated)

### P1. Position

**I agree with a cross-field-guided advancing front in (u,v) with a metric and topological cleanup, with one
change: the front advances *points over a Delaunay triangulation* (frontal-Delaunay in the cross-field L∞
norm, Remacle 2013), not quads (paving) and not triangle-to-quad transformations (Q-Morph).** All-quad comes
from the recombination + split FieldCraft already ships, and cleanup follows Reberol 2021.

Reasons:
1. **Closure cannot fail.** A frontal-Delaunay front always sits inside a valid constrained triangulation, so
   colliding fronts meet in triangles. Paving's collision cases and Q-Morph's edge-recovery and odd-closure
   cases do not exist. On the `docs`' own terms (robustness outranks features), this decides it.
2. **Alignment comes from the field, not the front.** Boundary-aligned Dirichlet data on the cross field gives
   the boundary-parallel rows analysts want from paving, and the field keeps them coherent where fronts meet.
3. **It reuses what exists.** `quad::cdt` (Bowyer–Watson, Sloan recovery, exact predicates), greedy
   recombination, the Catmull–Clark split, smoothing and the exact validity gate all stay. The new parts are
   the chart/metric layer, the cross field, the point placement rule, and cleanup.
4. **It has the best published CAD evidence** (S5.4).

What I would not build: direct paving (S1); Q-Morph as the first implementation (S2.1; keep as a possible
later point-placement alternative on the same triangulation, see P7); global-parametrization remeshers as the
primary shell mesher (S7: no prescribed boundary nodes, mixed-integer solves, mostly GPL code).

### P2. Pipeline

For each B-rep face set to be shell-meshed (a sheet body, or chosen faces of a solid):

1. **Edges first, once, in 3D** (exists): divide each B-rep edge with the shared `SizeField`
   (`size::divide_polyline`); both faces get the same nodes, so conformity across shared edges holds by
   construction.
2. **Global loop parity** (new): every face loop needs an even node count for the split. Each edge lies on
   two loops (closed shell) or one (free edge). Choosing which edges get ±1 division so that all loops are even
   at least cost is a minimum T-join on the face-adjacency graph (Edmonds and Johnson, *Matching, Euler tours
   and the Chinese postman*, Math. Prog. 5:88–124, 1973, DOI [10.1007/BF01580113](https://doi.org/10.1007/BF01580113)). It is always feasible on a
   closed shell (the sum of all loop counts is twice the edge total). A greedy shortest-path pairing of odd
   loops is enough to start; `size::loop_divisions` stays the single-loop case.
3. **Chart per face** (new, P3.1): native (u,v) when regular; otherwise a chart switch.
4. **Mapped faces first** (exists, extend): faces with four corners in (u,v) and compatible opposite counts get
   TFI in (u,v) with metric-aware spacing (S8.2). Cylinder walls, extrusion sides and fillets are mostly this
   case and come out perfect.
5. **Free faces** (new, P3.4): boundary-aligned cross field → frontal-Delaunay L∞ point placement at 2h → CDT →
   greedy recombination weighted by alignment → split → on-surface smoothing.
6. **Cleanup** (new, P3.6): disk-quadrangulation cavities and Canann/Kinney local operations, quality-monotone,
   bounded.
7. **Gate** (exists, extend): exact validity in (u,v); 3D quality report; error, never a tangled mesh.

### P3. Components

**P3.1 Chart abstraction** (`fieldcraft-mesh::surface_mesh::chart` or a new `shell` module; trait in `mesh`,
implementation in `geometry`, wired in `engine`, so `mesh` does not depend on `geometry`):
`eval(u,v) → [f64;3]`, `derivs(u,v) → (S_u, S_v)` (second derivatives for curvature sizing), `project(p,
hint) → (u,v)`, the face's loops as 3D node chains with their (u,v), periodicity and singular points. Native
charts come from SolidCraft's kernel surfaces (truck's parametric surfaces already have parameter search, used
in 5 SolidCraft kernel files); this needs a new **public** face-surface API in `solidcraft-kernel`, exposed
through `fieldcraft-geometry` without leaking SolidCraft types. Handling:
- **Periodic faces** (cylinder, torus, revolved): the seam edge appears twice in the loop; give its nodes
  u = 0 on one side and u = period on the other (unwrap by continuity along the loop). The domain becomes simply
  connected; seam nodes are the same 3D nodes, so conformity across the seam holds.
- **Poles** (sphere, cone apex, revolved curve touching the axis): switch chart. Mesh a cap around the pole in
  an orthographic tangent-plane projection (valid for a cap within a hemisphere), the rest in native (u,v);
  or, simpler and general, use a discrete chart (S3.7) for the whole face. Remacle–Geuzaine 2022 (S3.6) is the
  reference if we keep native (u,v) at poles instead.
- **Bad parametrizations** (S3.8: near-zero |S_u × S_v| inside the face, extreme metric ratios): the same
  discrete-chart fallback on the face's tessellation, which `BoundarySurface` already provides.
- **Faces from a tagged 3MF** (no B-rep surface): discrete chart only; new nodes projected onto the tessellation.

**P3.2 Metric.** `M(u,v) = JᵀJ / h(S(u,v))²` with J = [S_u S_v] (S3.1, S2.4). Edge lengths in the metric by the
same midpoint quadrature `size.rs` uses. For planes, cylinders and cones away from the apex, JᵀJ is constant
or nearly so after an affine rescale of (u,v); do that rescale first, which removes most anisotropy before any
metric code runs.

**P3.3 Size field.** Keep `SizeField` (isotropic, 3D point in, length out). Add a curvature cap
`h ≤ ρ_min·θ_max` (θ_max: turning angle per element, chordal-error form `h = √(8ρδ)` from Lo–Lau 1998) and
a proximity cap (Owen–Saigal 2000), combined by `min` with the user field and graded (|∇h| ≤ β).
Principal curvatures come from the chart's second fundamental form. The cross field's conformal scaling
(Reberol §3–§4) is a later refinement.

**P3.4 Frontal point placement.** Rebay's loop (S3.4) in (u,v): initial CDT of the half-resolution boundary
(as `quad::free` does now), triangles classified resolved/active/waiting by the L∞ circumradius *measured in
the local cross frame and the metric* (Remacle 2013 §5, generalized from det M^(1/4) to the full metric by
mapping the 3D tangent cross through J⁻¹), new point on the L∞ bisector of the active front edge, inserted by
Bowyer–Watson, layer by layer. Target size 2h (the split halves it). The Delaunay kernel stays the exact
Euclidean in-circle in the affinely rescaled (u,v) (Remacle 2013 §5 found L2 and L∞ kernels nearly identical);
the metric only decides where points go.

**P3.5 Validity.** In (u,v), exact `orient2d` on every element (exists). Given a chart with det J > 0 in the
face, a mesh valid in (u,v) maps to a non-folded surface mesh. Quality (scaled Jacobian, angles, warp, aspect)
is measured on the 3D quad: corner Jacobians against the surface normal at the corner.

**P3.6 Cleanup and smoothing.** Reberol §6.3 disk quadrangulations (enumerate small disk quadrangulations of a
cavity around an irregular vertex, pick the minimum irregularity score, accept only if the minimum quality does
not drop) plus Canann 1998 doublet/diagonal operations for boundary cases. Then smoothing: constrained
Laplacian (exists) → angle-based (Zhou–Shimada) → worst-element optimization with untangling (Escobar /
Ruiz-Gironés), nodes moving in (u,v), objective in 3D. Every loop bounded, with a "stopped early" report
(AGENTS.md).

**P3.7 Linear algebra placement.** The cross field needs a sparse symmetric solve per face (a few thousand to
a few hundred thousand unknowns). `mesh` is L1 and cannot use `solver` (L2). Either a small CG with Jacobi
preconditioning inside `mesh` (enough for a Laplace-type system, deterministic), or move a minimal sparse CG
into `model` (L0). I recommend the first: a 200-line deterministic CG local to the cross-field module.

**P3.8 Output.** A quad (and, for quad-dominant mode, quad + triangle) surface mesh with the B-rep face id on
every element, B-rep edge ids on boundary node chains, and the existing node-ordering convention, so sets and
surfaces "bound to SolidCraft face, edge, vertex and body names" (M3) resolve the same way they do for solids.
Quad-dominant mode is the same pipeline without the split (points at h, recombination only): useful when the
user prefers fewer elements and accepts some triangles.

### P4. What "done" looks like

Quality targets on our own corpus, measured on the 3D elements (each reported per mesh and frozen as fixtures
once met):

| Metric | Hard gate (mesh rejected or region flagged) | Target |
|---|---|---|
| Validity | every quad valid in (u,v) (exact) and every 3D corner Jacobian > 0 | — |
| Conformity | every B-rep edge's node chain identical in both faces; interior edges shared by exactly 2 elements | — |
| Min corner angle | ≥ 20° except at CAD corners whose input angle is smaller | ≥ 30° everywhere; ≥ 99% of quads with all angles in [45°, 135°] |
| Max corner angle | ≤ 160° | ≤ 150° |
| Scaled Jacobian (Knupp, min over corners) | > 0.1 | min ≥ 0.4, mean ≥ 0.9 (Reberol reports min SICN 0.44–0.53, mean 0.96–0.98) |
| Warp angle (Robinson) | ≤ 30° | ≤ 10° (driven by the curvature size cap) |
| Irregular interior vertices | — | ≤ 2% after cleanup (Reberol 0.27–0.74%; Delquad before cleanup 9–19%) |
| Size efficiency τ (Remacle §6.2) | — | ≥ 85% |
| Determinism | bit-identical mesh across platforms and thread counts | — |
| Robustness | every face of the corpus meshes or returns a located error; no panic under hostile parameters | ≥ 99% of corpus faces mesh without fallback |

Baseline first: run the current tri → quad split, ported to (u,v) (P6 step 2), over the corpus and record the
same table. The new placement and cleanup have to beat it on irregular vertices and min angle, or they do not
ship.

### P5. Verification

1. **Chart tests** (theory): round-trip eval/project on plane, cylinder, cone, sphere, torus and a SolidCraft
   NURBS face; seam unwrapping; pole detection; metric equals the analytic first fundamental form.
2. **Mesh invariants** (theory): conformity across shared edges, Euler characteristic per face and per shell,
   every node on its face (distance to S(u,v) zero by construction; to the true surface within kernel tolerance
   for discrete charts), face tags complete.
3. **Corpus** (our own SolidCraft recipes, per AGENTS.md): plates with holes and slots, filleted brackets,
   bosses and ribs, cylinders and cones with cut-outs, spheres and domes (poles), tori, revolved pressure-vessel
   heads, lofted/freeform NURBS skins, thin-walled boxes; a hostile series (tiny edges, sliver faces, near-
   degenerate parametrizations, acute corners). Report the P4 table per part.
4. **Shell FE convergence** (published benchmarks, S12): Scordelis–Lo roof, pinched cylinder, pinched
   hemisphere, each built in SolidCraft from the published description and meshed by *our free mesher*, not a
   structured grid, at 4–5 refinement levels. Acceptance: converges to the reference within the stated
   tolerance, and at the two finest levels the free-mesh error is at most 1.5× the error of a structured mapped
   mesh with the same node count (the structured mesh comes from our TFI). The 1.5× is a proposed threshold to
   be confirmed by the owner. Plus the MacNeal–Harder distorted-element patch tests on the element itself.
5. **Continuum shells** (M8): swept thin-walled parts, stack-direction check, through-thickness layer count,
   same convergence protocol with the hemisphere and cylinder built as solids.

### P6. Order of work

1. **Chart API** in `solidcraft-kernel` (public face-surface evaluation and parameter search) and
   `fieldcraft-geometry` (trait implementation, seams, poles). Done when the P5.1 tests pass.
2. **Port the shipped free mesher to (u,v) + metric**: half-resolution CDT and Ruppert refinement in the
   rescaled (u,v) with metric sizing, recombination, split, smoothing with 3D objective. This gives all-quad
   conforming meshes on curved faces early, and is the baseline.
3. **Loop parity (T-join) and curvature/proximity size field.**
4. **Mapped faces in (u,v)** (TFI, then submapping).
5. **Cross field** (Knöppel-style connection Laplacian with boundary alignment, local CG) and **frontal L∞
   placement** with alignment-weighted recombination.
6. **Cleanup** (disk quadrangulations, Canann operations) and on-surface smoothing/untangling.
7. **Verification suite** (P5.3–P5.4) and fixtures; then sweep to continuum shells (M8).

### P7. Risks and what happens if wrong

| Risk | Effect | Mitigation |
|---|---|---|
| Native parametrizations worse than expected (poles, seams, uneven NURBS knots) | Bad metric, folded charts, failures on real parts | Chart switch to discrete charts (S3.7) built on the existing tessellation; exact one-to-one check per chart |
| SolidCraft kernel cannot expose surfaces cleanly (cross-repo change) | Step 1 blocks | Discrete charts on the tessellation from day one work for every face (with projection onto the tessellation); native charts improve accuracy later |
| Split halves the resolution: features thinner than ~2h cannot be resolved | Errors on thin slots and slivers | Existing error path ("chords cross"); local refinement of the size field; quad-dominant mode without the split |
| Cross-field singularities placed poorly on small CAD features | Extra irregular vertices | Cleanup; Reberol's small-feature size map; accept and report |
| Boundary quantization (edge counts) fixes irregular vertices that no in-face operation can remove (Reberol §8) | Irregular count plateaus above target | Make the T-join edge-count choice alignment-aware later (count changes along cross-field-parallel edges are cheaper) |
| Metric in-circle or L∞ computations are not exact | Non-determinism or invalid meshes if used for decisions | Only placement uses floats; every topological decision is an exact predicate; validity gate is exact |

**If the call is wrong**, the two ways are: (a) frontal L∞ placement does not beat the ported split baseline
enough to justify itself on engineering parts; or (b) analysts want stricter boundary rows than a cross field
yields. Cost is bounded either way: the chart/metric layer, loop parity, size field, mapped faces, cleanup and
verification (steps 1–4, 6–7) are needed by every candidate method; only step 5 changes. For (a), keep the
baseline placement and invest in cleanup. For (b), implement Q-Morph's front transformation (S2.1, with QMCF's
field steering, S2.6) as an alternative placement on the same triangulation and compare on the same corpus.
The failure to avoid is building direct paving: its collision cases are where the 2000s methods failed, and
nothing since has made them safe.

---

## Licence summary of associated code (metadata only; no source read)

| Repo / code | Licence (as reported) | Status |
|---|---|---|
| Gmsh (Delquad, Blossom-Quad, quasi-structured quad pipeline, reparametrization) | GPL | FORBIDDEN |
| `nicopietroni/quadwild` | GPL-3.0 | FORBIDDEN |
| `cgg-bern/quadwild-bimdf` | GPL-3.0 | FORBIDDEN |
| `cgg-bern/libsatsuma` (BiMDF solvers) | MIT | READ OK |
| `cnr-isti-vclab/vcglib`, `cnr-isti-vclab/meshlab` | GPL-3.0 | FORBIDDEN |
| `hjwdzh/QuadriFlow` | BSD-3-Clause + enhancements paragraph (LICENSE.txt); bundled `3rd/` deps not checked | READ OK (top-level code only) |
| `wjakob/instant-meshes` | BSD-3-Clause + enhancements paragraph (LICENSE.txt); `ext/` deps not checked | READ OK (top-level code only) |
| `hcebke/libQEx` | GPL-3.0 | FORBIDDEN |
| libigl | MPL-2.0 core; `include/igl/copyleft/` (CoMISo/MIQ, CGAL, TetGen wrappers) GPL; GitHub reports GPL-3.0 | Core READ OK; `copyleft/` FORBIDDEN |
| CoMISo | GPL-3 | FORBIDDEN |
| `avaxman/Directional` | no licence file in the repo root, no GitHub licence metadata | NO LICENCE |
| `nmwsharp/geometry-central` (Knöppel 2013 fields) | MIT | READ OK |
| `pmp-library/pmp-library` | MIT | READ OK |
| `mlivesu/cinolib` | MIT | READ OK |
| `BrunoLevy/geogram` | BSD-3 except `third_party/` (see `literature.md`) | READ OK with exclusions |
| `GeometryCollective/boundary-first-flattening` (conformal parametrization) | MIT | READ OK |
| `wildmeshing/TriWild` | MPL-2.0 | READ OK |
| `alecjacobson/gptoolbox` | Apache-2.0 | READ OK |
| `MmgTools/mmg` (surface remeshing) | LGPL (README; COPYING.LESSER) | FORBIDDEN |
| QuadCover, MIQ (original), QGP, Lyon 2019/2021, QMCF, Q-Morph, paving | no public code found | — |

## Unverified items

- White and Kinney 1997 (S1.3): no DOI and no PDF fetched; pages 323–335 from Sandia's CUBIT reference list.
- Staten and Canann 1997, *Post refinement element shape improvement for quadrilateral meshes*, AMD-Vol. 220:
  not cited above for that reason.
- Palacios and Zhang 2007 (S4.3): DOI not re-checked.
- QMCF (S2.6): read from abstract snippets only; the ScienceDirect and ResearchGate pages returned 403.
- Ollivier-Gooch et al. 2025 (S1.5): SSRN preprint; only title and DOI verified.
- Remacle 2013 (S5.1): numbers are from the 2010 preprint; the journal version was not read.
- Benchmark reference values (S12): commonly quoted values; confirm from MacNeal–Harder 1985 and Belytschko et
  al. 1985 before freezing fixtures.
- `https://pages.cs.wisc.edu/~csverma/jaal.pdf` (Verma's Jaal all-quad generator) returned 404; not used.
- The UBC EDAMSurf thesis (advancing-layer surface meshing) was seen in search results only.
