(load-file "util.lisp")


(def! zk-not-small-order? (fn* [u v] (
        (def! first-doubling (last (last (zk-double u v))))
        (def! second-doubling (last (last 
            (zk-double (get first-doubling "u3") (get first-doubling "v3")))))
        (def! third-doubling (last (last 
            (zk-double (get second-doubling "u3") (get second-doubling "v3")))))
        (zk-nonzero? (get third-doubling "u3"))
        )
    )
)

(defmacro! zk-nonzero? (fn* [var] (
        (let* [inv (gensym)
               v1 (gensym)] (
        `(alloc ~inv (invert ~var))
        `(alloc ~v1 ~var)
        `(enforce  
            (scalar::one ~v1) 
            (scalar::one ~inv) 
            (scalar::one cs::one) 
         )
        )
    ))
))

(defmacro! zk-square (fn* [var] (
        (let* [v1 (gensym)
               v2 (gensym)] (
        `(alloc ~v1 ~var)
        `(def! output (alloc-input ~v2 (square ~var)))
        `(enforce  
            (scalar::one ~v1) 
            (scalar::one ~v1) 
            (scalar::one ~v2) 
         )
        `{ "v2" output }
        )
    ))
))

(defmacro! zk-mul (fn* [val1 val2] (
        (let* [v1 (gensym)
               v2 (gensym)
               var (gensym)] (
        `(alloc ~v1 ~val1)
        `(alloc ~v2 ~val2)
        `(def! result (alloc-input ~var (* ~val1 ~val2)))
        `(enforce  
            (scalar::one ~v1) 
            (scalar::one ~v2) 
            (scalar::one ~var) 
         )
        `{ "result" result }
        )
    ))
))

(defmacro! zk-witness (fn* [val1 val2] (
        (let* [u2 (gensym)
               v2 (gensym)
               u2v2 (gensym)
               EDWARDS_D (gensym)] (
        `(def! ~EDWARDS_D (alloc-const ~EDWARDS_D (scalar "2a9318e74bfa2b48f5fd9207e6bd7fd4292d7f6d37579d2601065fd6d6343eb1")))
        `(def! ~u2 (alloc ~u2 (get (nth (nth (zk-square ~val1) 0) 3) "v2")))
        `(def! ~v2 (alloc ~v2 (get (nth (nth (zk-square ~val2) 0) 3) "v2")))
        `(def! result (alloc-input ~u2v2 (get (last (last (zk-mul ~u2 ~v2))) "result")))        
        `(enforce  
            ((scalar::one::neg ~u2) (scalar::one ~v2))
            (scalar::one cs::one)
            ((scalar::one cs::one) (~EDWARDS_D ~u2v2))
         )
        `{ "result" result }
        )
    ))
))

(defmacro! zk-double (fn* [val1 val2] (
        (let* [u (gensym)
               v (gensym)
               u3 (gensym)
               v3 (gensym)
               T (gensym)
               A (gensym)
               C (gensym)
               EDWARDS_D (gensym)] (
        `(def! ~EDWARDS_D (alloc-const ~EDWARDS_D (scalar "2a9318e74bfa2b48f5fd9207e6bd7fd4292d7f6d37579d2601065fd6d6343eb1")))
        `(def! ~u (alloc ~u ~val1))
        `(def! ~v (alloc ~v ~val2))
        `(def! ~T (alloc ~T (* (+ ~val1 ~val2) (+ ~val1 ~val2))))
        `(def! ~A (alloc ~A (* ~u ~v)))
        `(def! ~C (alloc ~C (* (square ~A) ~EDWARDS_D)))
        `(def! ~u3 (alloc-input ~u3 (/ (double ~A) (+ scalar::one ~C))))
        `(def! ~v3 (alloc-input ~v3 (/ (- ~T (double ~A)) (- scalar::one ~C))))
        `(enforce  
            ((scalar::one ~u) (scalar::one ~v))
            ((scalar::one ~u) (scalar::one ~v))
            (scalar::one ~T)
         )
         `(enforce  
            (~EDWARDS_D ~A)
            (scalar::one ~A)
            (scalar::one ~C)
         )
         `(enforce  
            ((scalar::one cs::one) (scalar::one ~C))
            (scalar::one ~u3)
            ((scalar::one ~A) (scalar::one ~A))    
         )
         `(enforce  
            ((scalar::one cs::one) (scalar::one::neg ~C))
            (scalar::one ~v3)
            ((scalar::one ~T) (scalar::one::neg ~A) (scalar::one::neg ~A))    
         )    
        { "u3" u3, "v3" v3 }
        )
    ))
))

(defmacro! conditionally-select (fn* [val1 val2 val3] (
        (let* [u-prime (gensym)
               v-prime (gensym)
               u (gensym)
               v (gensym)
               condition (gensym)
               ] (
            `(def! ~u (alloc ~u ~val1))
            `(def! ~v (alloc ~v ~val2))
            `(def! ~condition (alloc ~condition ~val3))
            `(def! ~u-prime (alloc-input ~u-prime (* ~u ~condition)))
            `(def! ~v-prime (alloc-input ~v-prime (* ~v ~condition)))
            `(enforce
                (scalar::one ~u)
                (scalar::one ~condition)
                (scalar::one ~u-prime)
             )
            `(enforce
                (scalar::one ~v)
                (scalar::one ~condition)
                (scalar::one ~v-prime)
             )
             { "u-prime" u-prime, "v-prime" v-prime }
        )
))))

(defmacro! jj-add (fn* [param1 param2 param3 param4]
    (let* [u1 (gensym) v1 (gensym) u2 (gensym) v2 (gensym)
           EDWARDS_D (gensym) U (gensym) A (gensym) B (gensym)
           C (gensym) u3 (gensym) v3 (gensym)] (
        `(def! ~u1 (alloc ~u1 ~param1))
        `(def! ~v1 (alloc ~v1 ~param2))
        `(def! ~u2 (alloc ~u2 ~param3))
        `(def! ~v2 (alloc ~v2 ~param4)) 
        `(def! ~EDWARDS_D (alloc-const ~EDWARDS_D (scalar "2a9318e74bfa2b48f5fd9207e6bd7fd4292d7f6d37579d2601065fd6d6343eb1")))
        `(def! ~U (alloc ~U (* (+ ~u1 ~v1) (+ ~u2 ~v2))))
        `(def! ~A (alloc ~A (* ~v2 ~u1)))
        `(def! ~B (alloc ~B (* ~u2 ~v1)))
        `(def! ~C (alloc ~C (* ~EDWARDS_D (* ~A ~B))))
        `(def! ~u3 (alloc-input ~u3 (/ (+ ~A ~B) (+ scalar::one ~C))))
        `(def! ~v3 (alloc-input ~v3 (/ (- (- ~U ~A) ~B) (- scalar::one ~C))))        
  `(enforce  
    ((scalar::one ~u1) (scalar::one ~v1))
    ((scalar::one ~u2) (scalar::one ~v2))
    (scalar::one ~U)
   )
  `(enforce
    (~EDWARDS_D ~A)
    (scalar::one ~B)
    (scalar::one ~C)
   )
  `(enforce
    ((scalar::one cs::one)(scalar::one ~C))
    (scalar::one ~u3)
    ((scalar::one ~A) (scalar::one ~B))
   )
  `(enforce
    ((scalar::one cs::one) (scalar::one::neg ~C))
    (scalar::one ~v3)
    ((scalar::one ~U) (scalar::one::neg ~A) (scalar::one::neg ~B))
   )   
   { "u3" u3, "v3" v3 }
  )  
)
))

;; cs.enforce(
;;     || "boolean constraint",
;;     |lc| lc + CS::one() - var,
;;     |lc| lc + var,
;;     |lc| lc,
;; );
(defmacro! zk-boolean (fn* [val] (
        (let* [var (gensym)] (
            `(alloc ~var ~val)
            `(enforce
                (scalar::one cs::one) (scalar::one ~var)
                (scalar::one ~var)
                ()
             )
        )
))))

(def! jj-mul (fn* [u v b] (
    (def! result (unpack-bits b))
    (eval (map zk-boolean result))
    (def! val (last (last (zk-double param-u param-v))))
    (def! acc 1)
    (dotimes (count result) (            
        (def! u3 (get val "u3"))
        (def! v3 (get val "v3"))
        (def! val (last (last (zk-double u3 v3))))     
        (def! r (nth result acc))
        (def! cond-result (last (last (conditionally-select u3 v3 r))))
        (def! u-prime (get cond-result "u-prime"))
        (def! v-prime (get cond-result "v-prime"))
        (def! add-result (last (jj-add u3 v3 u-prime v-prime)))               
        (def! acc (i+ acc 1))
        (println acc u3 v3 u-prime v-prime cond-result add-result)        
    ))
)))

(def! param3 (scalar "0000000000000000000000000000000000000000000000000000000000000000"))
(def! param-u (scalar "273f910d9ecc1615d8618ed1d15fef4e9472c89ac043042d36183b2cb4d7ef51"))
(def! param-v (scalar "466a7e3a82f67ab1d32294fd89774ad6bc3332d0fa1ccd18a77a81f50667c8d7"))
(def! param1 (scalar 42))
(prove 
  (
    ;; (println (zk-square param1))
    (jj-mul param-u param-v param3)
  )
)

;; following some examples 
;; (def! alloc-u (alloc "alloc-u" param-u))
;;     (def! alloc-v (alloc "alloc-v" param-v))
;;     (def! condition (alloc "condition" param3))
;;     (println 'conditionally_select 
;;         (conditionally_select alloc-u alloc-v condition))
;; (println (zk-mul param1 param2))
;; (def! param1 (scalar 3))
;; (def! param2 (scalar 9))
;; (println (zk-square param1))
;; (println (zk-mul param1 param2))
;; (println 'witness (zk-witness param-u param-v))
;; (println 'double (last (last (zk-double param-u param-v))))
;; (println 'nonzero (zk-nonzero? param3))    
;; (println 'not-small-order? (zk-not-small-order? param-u param-v))
