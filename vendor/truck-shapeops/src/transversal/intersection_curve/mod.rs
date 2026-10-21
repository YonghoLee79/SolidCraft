use truck_base::cgmath64::*;
use truck_geometry::prelude::*;
use truck_meshalgo::prelude::*;

/// polyline base ntersection curve with parameter
#[derive(Debug, Clone, derive_more::Deref, derive_more::DerefMut)]
pub struct IntersectionCurveWithParameters<S> {
    #[deref]
    #[deref_mut]
    ic: IntersectionCurve<PolylineCurve<Point3>, S>,
    params0: PolylineCurve<Point2>,
    params1: PolylineCurve<Point2>,
}

impl<S> From<IntersectionCurveWithParameters<S>> for IntersectionCurve<PolylineCurve<Point3>, S> {
    fn from(a: IntersectionCurveWithParameters<S>) -> Self { a.ic }
}

impl<S> IntersectionCurveWithParameters<S>
where S: ParametricSurface3D + SearchNearestParameter<D2, Point = Point3>
{
    /// SolidCraft: construct a projected curve, returning `None` for degenerate input.
    pub fn try_new(
        surface0: S,
        surface1: S,
        poly: PolylineCurve<Point3>,
        tol: f64,
    ) -> Option<Self> {
        // SolidCraft: a point contact is not a curve. Reject it before indexing the
        // endpoints, and reject collapsed/non-finite segments before normalizing.
        if poly.len() < 2 || poly.windows(2).any(|p| {
            let length2 = (p[1] - p[0]).magnitude2();
            !length2.is_finite() || length2 <= 0.0
        }) {
            return None;
        }
        let mut polyline = PolylineCurve(Vec::new());
        let mut params0 = PolylineCurve(Vec::new());
        let mut params1 = PolylineCurve(Vec::new());
        for p in poly.windows(2) {
            let n = (p[1] - p[0]).normalize();
            let (q, p0, p1) = project_onto_curve(&surface0, &surface1, p[0], n, tol)?;
            polyline.push(q);
            params0.push(p0);
            params1.push(p1);
        }
        let (q, p0, p1) = if poly[0].near(&poly[poly.len() - 1]) {
            (polyline[0], params0[0], params1[0])
        } else {
            let n = (poly[poly.len() - 1] - poly[poly.len() - 2]).normalize();
            project_onto_curve(&surface0, &surface1, poly[poly.len() - 1], n, tol)?
        };
        polyline.push(q);
        params0.push(p0);
        params1.push(p1);
        Some(Self {
            ic: IntersectionCurve::new_unchecked(
                Box::new(surface0),
                Box::new(surface1),
                polyline,
                tol,
            ),
            params0,
            params1,
        })
    }
}

/// SolidCraft: a point of the leading polyline moved onto the intersection of the two
/// surfaces, in the plane through it across the polyline (`direction`), as truck does; when
/// that does not converge, in the plane through it across the intersection curve itself.
///
/// The polyline is where the two faces' meshes meet, so it lies up to the mesh tolerance off
/// the surfaces, and on a face that is small beside that tolerance it can zigzag across the
/// curve: its direction there is nearly along the curve's normal plane, the plane across it
/// nearly contains the curve, and Newton's method slides along the curve, off the face onto the
/// surface's continuation past its knot range, and diverges. One such point failed the whole
/// curve, and with it the boolean. The curve's own direction (the cross product of the two
/// surfaces' normals at the point's nearest parameters) gives the plane the step needs; a
/// point it moves farther than ten tolerances is not taken.
fn project_onto_curve<S>(
    surface0: &S,
    surface1: &S,
    point: Point3,
    direction: Vector3,
    tol: f64,
) -> Option<(Point3, Point2, Point2)>
where
    S: ParametricSurface3D + SearchNearestParameter<D2, Point = Point3>,
{
    if let Some(found) = double_projection(surface0, None, surface1, None, point, direction, 100) {
        return Some(found);
    }
    let uv0 = surface0.search_nearest_parameter(point, None, 10)?;
    let uv1 = surface1.search_nearest_parameter(point, None, 10)?;
    let along = surface0.normal(uv0.0, uv0.1).cross(surface1.normal(uv1.0, uv1.1));
    let length = along.magnitude();
    if !(length > 1.0e-3 && length.is_finite()) {
        return None;
    }
    let along = if along.dot(direction) < 0.0 { -along } else { along } / length;
    double_projection(surface0, Some(uv0), surface1, Some(uv1), point, along, 100)
        .filter(|(q, _, _)| q.distance(point) <= 10.0 * tol)
}

impl<S> ParametricCurve for IntersectionCurveWithParameters<S>
where S: ParametricSurface3D + SearchNearestParameter<D2, Point = Point3>
{
    type Point = Point3;
    type Vector = Vector3;
    #[inline(always)]
    fn subs(&self, t: f64) -> Point3 { self.ic.subs(t) }
    #[inline(always)]
    fn der(&self, t: f64) -> Vector3 { self.ic.der(t) }
    #[inline(always)]
    fn der2(&self, t: f64) -> Vector3 { self.ic.der2(t) }
    #[inline(always)]
    fn parameter_range(&self) -> ParameterRange { self.ic.parameter_range() }
}

impl<S> BoundedCurve for IntersectionCurveWithParameters<S> where S: ParametricSurface3D + SearchNearestParameter<D2, Point = Point3> {}

impl<S> ParameterDivision1D for IntersectionCurveWithParameters<S>
where S: ParametricSurface3D + SearchNearestParameter<D2, Point = Point3>
{
    type Point = Point3;
    #[inline(always)]
    fn parameter_division(&self, range: (f64, f64), tol: f64) -> (Vec<f64>, Vec<Point3>) {
        self.ic.parameter_division(range, tol)
    }
}

impl<S> Cut for IntersectionCurveWithParameters<S>
where S: ParametricSurface3D + SearchNearestParameter<D2, Point = Point3>
{
    #[inline(always)]
    fn cut(&mut self, t: f64) -> Self {
        Self {
            ic: self.ic.cut(t),
            params0: self.params0.cut(t),
            params1: self.params1.cut(t),
        }
    }
}

impl<S: Clone> Invertible for IntersectionCurveWithParameters<S> {
    fn invert(&mut self) {
        self.ic.invert();
        self.params0.invert();
        self.params1.invert();
    }
}

impl<S> SearchParameter<D1> for IntersectionCurveWithParameters<S>
where S: ParametricSurface3D + SearchNearestParameter<D2, Point = Point3>
{
    type Point = Point3;
    fn search_parameter<H: Into<SPHint1D>>(
        &self,
        point: Point3,
        hint: H,
        trials: usize,
    ) -> Option<f64> {
        self.ic.search_parameter(point, hint, trials)
    }
}

impl<S> SearchNearestParameter<D1> for IntersectionCurveWithParameters<S>
where S: ParametricSurface3D + SearchNearestParameter<D2, Point = Point3>
{
    type Point = Point3;
    fn search_nearest_parameter<H: Into<SPHint1D>>(
        &self,
        point: Point3,
        hint: H,
        trials: usize,
    ) -> Option<f64> {
        self.ic.search_nearest_parameter(point, hint, trials)
    }
}

pub fn intersection_curves<S>(
    surface0: S,
    polygon0: &PolygonMesh,
    surface1: S,
    polygon1: &PolygonMesh,
    tol: f64,
) -> Vec<(
    PolylineCurve<Point3>,
    Option<IntersectionCurveWithParameters<S>>,
)>
where
    S: ParametricSurface3D + SearchNearestParameter<D2, Point = Point3>,
{
    let interferences = polygon0.extract_interference(polygon1);
    let polylines = super::polyline_construction::construct_polylines(&interferences);
    polylines
        .into_iter()
        .map(|polyline| {
            let curve = IntersectionCurveWithParameters::try_new(
                surface0.clone(),
                surface1.clone(),
                polyline.clone(),
                tol,
            );
            (polyline, curve)
        })
        .collect()
}

#[cfg(test)]
mod tests;
