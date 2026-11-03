package u;

import static u1.E.CASE;
import static u1.S.called;

import u1.Ann;
import u1.ArrNew;
import u1.Cast;
import u1.Catch;
import u1.ClsLit;
import u1.Diamond;
import u1.Ext;
import u1.Impl;
import u1.Inst;
import u1.Lam;
import u1.Local;
import u1.MRef;
import u1.Name;
import u1.Perm;
import u1.Pkg;
import u1.Wild;
import u1.u;

class T<TP> extends Ext implements Impl {
  @Ann(Name = 1)
  int Field;

  void Meth() {}

  class Cls {}

  void f(int Param, Object x) {
    int Local = 0;
    Local++;
    Lbl:
    for (; ; ) break Lbl;
    x.MemberSel.y();
    Pkg.a.b();
    Runnable r = MRef::Member;
    java.util.function.IntUnaryOperator o = Lam -> Lam;
    Object a = ClsLit.class;
    Object b = ArrNew[]::new;
    Object d = new Diamond<>();
    Object e = (Cast) x;
    boolean g = x instanceof Inst;
    java.util.List<? extends Wild> h;
    try {
    } catch (Catch z) {
    }
    switch (x) {
      case CASE -> {}
      default -> {}
    }
    called();
  }
}

sealed interface S permits Perm {}
