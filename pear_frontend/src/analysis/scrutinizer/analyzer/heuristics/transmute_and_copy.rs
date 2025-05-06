use rustc_middle::mir::{visit::Visitor, Body, Location, Mutability, Rvalue};
use rustc_middle::mir::{
    CastKind, CopyNonOverlapping, Local, NonDivergingIntrinsic, Operand, Statement, StatementKind,
};
use rustc_middle::ty::{self, Ty, TyCtxt, TypeSuperVisitable, TypeVisitable, TypeVisitor};

use std::ops::ControlFlow;

struct TransmuteAndCopyVisitor<'tcx> {
    tcx: TyCtxt<'tcx>,
    has_transmute: bool,
    has_copy: bool,
    important_args: Vec<Local>,
}

pub trait HasTransmuteAndCopy<'tcx> {
    fn has_transmute_or_copy(&self, tcx: TyCtxt<'tcx>, important_args: Vec<Local>) -> bool;
}

impl<'tcx> HasTransmuteAndCopy<'tcx> for Body<'tcx> {
    fn has_transmute_or_copy(&self, tcx: TyCtxt<'tcx>, important_args: Vec<Local>) -> bool {
        let mut ptr_deref_visitor = TransmuteAndCopyVisitor {
            tcx,
            has_transmute: false,
            has_copy: false,
            important_args,
        };
        ptr_deref_visitor.visit_body(self);
        ptr_deref_visitor.has_transmute || ptr_deref_visitor.has_copy
    }
}

impl<'a, 'tcx> Visitor<'tcx> for TransmuteAndCopyVisitor<'tcx> {
    fn visit_rvalue(&mut self, rvalue: &Rvalue<'tcx>, location: Location) {
        if let Rvalue::Cast(CastKind::Transmute, _, to) = rvalue {
            if contains_mut_ref(to, self.tcx) {
                self.has_transmute = true;
            }
        }
        self.super_rvalue(rvalue, location);
    }

    fn visit_statement(&mut self, statement: &Statement<'tcx>, location: Location) {
        if let StatementKind::Intrinsic(box NonDivergingIntrinsic::CopyNonOverlapping(
            CopyNonOverlapping { src, .. },
        )) = &statement.kind
        {
            if let Operand::Copy(place) | Operand::Move(place) = src
                && self.important_args.contains(&place.local)
            // This depends on the fact that `CopyNonoverlapping` operates directly on the arguments in the intrinsic.
            {
                self.has_copy = true;
            }
        }
        self.super_statement(statement, location);
    }
}

pub fn contains_mut_ref<'tcx>(ty: &Ty<'tcx>, tcx: TyCtxt<'tcx>) -> bool {
    struct ContainsMutRefVisitor<'tcx> {
        tcx: TyCtxt<'tcx>,
        has_mut_ref: bool,
    }

    impl<'tcx> TypeVisitor<TyCtxt<'tcx>> for ContainsMutRefVisitor<'tcx> {
        type BreakTy = ();

        fn visit_ty(&mut self, t: Ty<'tcx>) -> ControlFlow<Self::BreakTy> {
            if let ty::TyKind::Adt(adt_def, substs) = t.kind() {
                for field in adt_def.all_fields() {
                    field.ty(self.tcx, substs).visit_with(self)?;
                }
            }

            if let Some(Mutability::Mut) = t.ref_mutability() {
                self.has_mut_ref = true;
            }
            t.super_visit_with(self)
        }
    }

    let mut visitor = ContainsMutRefVisitor {
        tcx,
        has_mut_ref: false,
    };
    ty.visit_with(&mut visitor);
    visitor.has_mut_ref
}
