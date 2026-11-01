package p;

import javax.annotation.processing.Generated;
import lombok.Data;
import lombok.Builder;
import org.springframework.beans.factory.annotation.Value;

@Generated("tool")
class Made {
    class Inner {}
    static class Deeper { class Deepest {} }
}

@javax.annotation.Generated(value = "other")
class AlsoMade {}

@Data
class Bean {
    int x;
}

class Fields {
    @lombok.Getter int x;
    @Builder.Default int y = 1;
}

class SpringValue {
    @Value("${x}") int x;
}

@Override
class NotGenerated {}
