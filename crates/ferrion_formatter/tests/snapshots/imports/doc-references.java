package p;

import u.Link;
import u.LinkM;
import u.LinkPlain;
import u.LinkParam;
import u.LinkArr;
import u.LinkLabel;
import u.Gen;
import u.QualFirst;
import u.QualLast;
import u.Value;
import u.ValueOnly;
import u.Code;
import u.Literal;
import u.Prose;
import u.See;
import u.SeeLabel;
import u.Thr;
import u.Exc;
import u.Uses;
import u.Provides;
import u.ParamDoc;
import u.Field;
import static u.S.member;
import static u.S.called;

/**
 * Prose. {@link Link} {@link LinkM#m} {@linkplain LinkPlain text} {@link #m(LinkParam)} {@link
 * LinkArr[]} {@link String LinkLabel} {@link Gen<String>} {@link QualFirst.Inner} {@link
 * a.QualLast} {@value Value#X} {@value ValueOnly} {@code Code} {@literal Literal} {@link member}
 * {@link called()}
 *
 * @param <Q> ParamDoc
 * @see See
 * @see String SeeLabel
 * @throws Thr when
 * @exception Exc when
 * @uses Uses
 * @provides Provides
 */
class T<Q> {
  /** {@link Field} */
  int f;
}
