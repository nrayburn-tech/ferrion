package p;

import lombok.*;
import java.util.List;

@Deprecated
class Plain {
    @Override public String toString() { return ""; }
}

@Getter
class Annotated {}

@List
class SingleImportWins {}
