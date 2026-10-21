#![allow(clippy::unwrap_used)]
use super::*;
use std::collections::HashSet;
use truck_meshalgo::prelude::{PolygonMesh, PolylineCurve};
use truck_meshalgo::tessellation::{Deflection, RoundMeshableShape};

type MeshShell = truck_topology::Shell<mt::Point3, PolylineCurve<mt::Point3>, Option<PolygonMesh>>;

fn shared_intersection_prism() -> mt::Shell {
    let k = 0.5522847498307936;
    let controls = [
        [[1., 0.], [1., k], [k, 1.], [0., 1.]],
        [[0., 1.], [-k, 1.], [-1., k], [-1., 0.]],
        [[-1., 0.], [-1., -k], [-k, -1.], [0., -1.]],
        [[0., -1.], [k, -1.], [1., -k], [1., 0.]],
    ];
    let curves: Vec<_> = controls
        .iter()
        .map(|c| mt::BSplineCurve::new(mt::KnotVec::bezier_knot(3), c.iter().map(|p| mt::Point3::new(p[0] * 3., p[1] * 3., 0.)).collect()))
        .collect();
    let vertices: Vec<_> = curves.iter().map(|c| mt::Vertex::new(c.subs(0.))).collect();
    let edges: Vec<_> =
        curves.into_iter().enumerate().map(|(i, c)| mt::Edge::new(&vertices[i], &vertices[(i + 1) % 4], mt::Curve::BSplineCurve(c))).collect();
    let face = mt::Face::new(
        vec![edges.into()],
        mt::Surface::Plane(mt::Plane::new(mt::Point3::new(0., 0., 0.), mt::Point3::new(1., 0., 0.), mt::Point3::new(0., 1., 0.))),
    );
    let solid: mt::Solid = mt::builder::tsweep(&face, mt::Vector3::new(0., 0., 4.));
    let shell = &solid.boundaries()[0];
    let mut lookup: HashMap<_, usize> = HashMap::new();
    let mut incidence: Vec<(mt::Curve, Vec<mt::Surface>)> = Vec::new();
    for face in shell.face_iter() {
        for edge in face.edge_iter() {
            if let Some(index) = lookup.get(&edge.id()).copied() {
                incidence[index].1.push(face.surface());
            } else {
                lookup.insert(edge.id(), incidence.len());
                incidence.push((edge.curve(), vec![face.surface()]));
            }
        }
    }
    // Use the two actual incident supports. The cap curves retain their original
    // leaders and ranges; the straight vertical edges remain ordinary curves.
    shell.mapped(|p| *p, |curve| {
        if let mt::Curve::BSplineCurve(leader) = curve && leader.degree() == 3 {
            let (_, supports) = incidence.iter().find(|(candidate, _)| matches!(candidate, mt::Curve::BSplineCurve(c) if c.knot_vec() == leader.knot_vec() && c.control_points() == leader.control_points())).unwrap();
            assert_eq!(supports.len(), 2);
            let plane = supports.iter().find(|s| matches!(s, mt::Surface::Plane(_))).unwrap();
            let spline = supports.iter().find(|s| matches!(s, mt::Surface::BSplineSurface(_))).unwrap();
            let result = mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(Box::new(plane.clone()), Box::new(spline.clone()), mt::Leader::BSpline(leader.clone()), 1e-8));
            for i in 0..=32 {
                let t = i as f64 / 32.;
                let expected = curve.subs(t);
                let actual = result.try_subs(t).unwrap();
                assert!((expected.x - actual.x).abs().max((expected.y - actual.y).abs()).max((expected.z - actual.z).abs()) < 1e-10);
            }
            result
        } else {
            curve.clone()
        }
    }, |surface| surface.clone())
}

fn signature(shell: &MeshShell) -> String {
    let mut out = String::new();
    for face in shell.face_iter() {
        out.push_str(&format!("orientation={}", face.orientation()));
        let mesh = face.surface().unwrap();
        assert!(mesh.faces().triangle_iter().next().is_some());
        for p in mesh.positions() {
            out.push_str(&format!("p{:x},{:x},{:x}", p.x.to_bits(), p.y.to_bits(), p.z.to_bits()));
        }
        for n in mesh.normals() {
            out.push_str(&format!("n{:x},{:x},{:x}", n.x.to_bits(), n.y.to_bits(), n.z.to_bits()));
        }
        for uv in mesh.uv_coords() {
            out.push_str(&format!("uv{:x},{:x}", uv.x.to_bits(), uv.y.to_bits()));
        }
        out.push_str(&format!("f{:?}", mesh.faces()));
        for edge in face.edge_iter() {
            out.push_str(&format!("edge_orientation={}", edge.orientation()));
            for p in edge.curve().iter() {
                out.push_str(&format!("{:x},{:x},{:x}", p.x.to_bits(), p.y.to_bits(), p.z.to_bits()));
            }
        }
    }
    out
}

#[test]
fn shared_intersection_edges_preserve_mesh_bits_and_budget() {
    let raw = shared_intersection_prism();
    assert!(mt::Solid::try_new(vec![raw.clone()]).is_ok());
    let unique: HashSet<_> = raw.edge_iter().map(|e| e.id()).collect();
    assert!(raw.edge_iter().count() > unique.len());
    let intersection: HashSet<_> = raw.edge_iter().filter(|e| matches!(e.curve(), mt::Curve::IntersectionCurve(_))).map(|e| e.id()).collect();
    assert_eq!(intersection.len(), 8);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
    for chord in [0.01, 0.08, 0.4] {
        for angle in [f64::INFINITY, 20f64.to_radians()] {
            let d = Deflection { chord, angle, floor: if angle.is_finite() { chord * 1e-3 } else { 0. }, rulings: true };
            let reference = pool.install(|| raw.robust_triangulation_within(d));
            let expected = signature(&reference);
            for cap in [0, 1, MAX_GRIDS] {
                let budget = Arc::new(MeshBudget::new(cap));
                let mapped = raw.mapped(|p| *p, |curve| MeshCurve::new(curve.clone(), budget.clone()), |surface| surface.clone());
                let actual = pool.install(|| mapped.robust_triangulation_within(d));
                assert_eq!(signature(&actual), expected, "chord={chord} angle={angle} cap={cap}");
                assert!(budget.cells.lock().unwrap().len() <= cap.min(4));
                assert!(budget.stats().0 <= cap && budget.stats().1 <= cap);
                if cap >= 4 {
                    assert_eq!(budget.cells.lock().unwrap().len(), 4);
                    assert_eq!(budget.stats().2, 0);
                }
            }
        }
    }
}
#[test]
fn failed_first_support_keeps_rejection_and_leaves_second_unprepared() {
    let rational = |first: f64| {
        let control_points = (0..2)
            .map(|i| {
                (0..2)
                    .map(|j| {
                        let w = if i == 0 && j == 0 { first } else { 1. };
                        mt::Vector4::new(i as f64 * w, j as f64 * w, (i * j) as f64 * w, w)
                    })
                    .collect()
            })
            .collect();
        mt::Surface::NurbsSurface(mt::NurbsSurface::new(mt::BSplineSurface::new(
            (mt::KnotVec::bezier_knot(1), mt::KnotVec::bezier_knot(1)),
            control_points,
        )))
    };
    let leader = mt::Leader::Polyline(mt::PolylineCurve(vec![mt::Point3::new(1., 1., 1.), mt::Point3::new(2., 1., 1.)]));
    let raw = mt::Curve::IntersectionCurve(mt::IntersectionCurve::new_unchecked(Box::new(rational(f64::NAN)), Box::new(rational(1.)), leader, 0.01));
    assert!(raw.try_subs(0.5).is_none());
    let budget = Arc::new(MeshBudget::new(2));
    let prepared = MeshCurve::new(raw, budget.clone());
    assert!(prepared.try_subs(0.5).is_none());
    assert!(prepared.grids.0.lock().unwrap().cached.upgrade().is_some());
    assert!(prepared.grids.1.lock().unwrap().cached.upgrade().is_none());
    assert_eq!(budget.cells.lock().unwrap().len(), 1);
}
